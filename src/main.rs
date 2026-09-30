use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::SeqCst};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const MAX_AGE_MS: u128 = 7 * 24 * 60 * 60 * 1000;
const MAX_PAUSE_BYTES: usize = 2 * 1024 * 1024; // cap so a paused noisy child can't eat RSS
const LOG_TAIL_BYTES: u64 = 256 * 1024; // window when tailing logs for display
const TICK: Duration = Duration::from_millis(100);

// ── Context ──
// State is per-user under /tmp (not $TMPDIR: on macOS it differs between GUI
// and SSH sessions, which would split state in two). One flat dir per cwd,
// named by a hash of the path: <base>/<cwd-hash>/<name>/.

struct Ctx {
    base: PathBuf,
    dir: PathBuf,
    cwd: PathBuf,
}
static CTX: OnceLock<Ctx> = OnceLock::new();
fn ctx() -> &'static Ctx {
    CTX.get().expect("ctx")
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

// ── Terminal ──
// Every TUI path records what it changed here, and restore_terminal() undoes
// it on any exit (normal, signal, panic), so a dying whiskd never leaves the
// terminal in raw mode / alt screen / a scroll region.

static ORIG_TERMIOS: Mutex<Option<libc::termios>> = Mutex::new(None);
static SCROLL_REGION: AtomicBool = AtomicBool::new(false);
static ALT_SCREEN: AtomicBool = AtomicBool::new(false);
static SIGNAL: AtomicI32 = AtomicI32::new(0);

fn out(bytes: &[u8]) {
    let mut o = io::stdout().lock();
    let _ = o.write_all(bytes);
    let _ = o.flush();
}

fn is_tty(fd: i32) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

fn term_size() -> (usize, usize) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 && ws.ws_col > 0 {
            return (ws.ws_row as usize, ws.ws_col as usize);
        }
    }
    (24, 80)
}

// Raw input but keep output processing (ONLCR), like Node's setRawMode, so
// log newlines still return the carriage.
fn raw_on() {
    if !is_tty(0) {
        return;
    }
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) != 0 {
            return;
        }
        let mut orig = ORIG_TERMIOS.lock().unwrap_or_else(|e| e.into_inner());
        if orig.is_none() {
            *orig = Some(t);
        }
        t.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
        t.c_cflag |= libc::CS8;
        t.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        libc::tcsetattr(0, libc::TCSADRAIN, &t);
    }
}

fn raw_off() {
    if let Some(t) = ORIG_TERMIOS.lock().unwrap_or_else(|e| e.into_inner()).take() {
        unsafe { libc::tcsetattr(0, libc::TCSADRAIN, &t) };
    }
}

fn setup_scroll_region(rows: usize) {
    SCROLL_REGION.store(true, SeqCst);
    out(format!("\x1b[1;{}r\x1b[{};1H", rows - 1, rows - 1).as_bytes());
}

fn reset_scroll_region() {
    if SCROLL_REGION.swap(false, SeqCst) {
        let rows = term_size().0;
        out(format!("\x1b[1;{rows}r\x1b[{rows};1H\x1b[2K").as_bytes());
    }
}

fn enter_alt() {
    ALT_SCREEN.store(true, SeqCst);
    out(b"\x1b[?1049h\x1b[?25l");
    raw_on();
}

fn leave_alt() {
    if ALT_SCREEN.swap(false, SeqCst) {
        out(b"\x1b[?25h\x1b[?1049l");
    }
    raw_off();
}

fn restore_terminal() {
    reset_scroll_region();
    leave_alt();
}

fn exit(code: i32) -> ! {
    restore_terminal();
    let _ = io::stdout().flush();
    std::process::exit(code)
}

fn die(msg: &str) -> ! {
    eprintln!("whiskd: {msg}");
    exit(1)
}

// TUIs catch these so they can restore the terminal before dying. Detached
// children live in their own session and are unaffected.
extern "C" fn on_signal(sig: libc::c_int) {
    SIGNAL.store(sig, SeqCst);
}

fn catch_signals() {
    for s in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        unsafe { libc::signal(s, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    }
}

fn check_signal() {
    let s = SIGNAL.load(SeqCst);
    if s != 0 {
        exit(128 + s);
    }
}

// Waits up to `ms` for a keypress. Without a tty there are no keys; just wait.
fn read_key(ms: i32) -> Option<Vec<u8>> {
    if !is_tty(0) {
        sleep(Duration::from_millis(ms as u64));
        return None;
    }
    let mut pfd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
    if unsafe { libc::poll(&mut pfd, 1, ms) } <= 0 {
        return None;
    }
    let mut buf = [0u8; 64];
    let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
    (n > 0).then(|| buf[..n as usize].to_vec())
}

fn is_up(k: &[u8]) -> bool {
    k.starts_with(b"\x1b[A") || k[0] == b'k'
}
fn is_down(k: &[u8]) -> bool {
    k.starts_with(b"\x1b[B") || k[0] == b'j'
}
fn is_esc(k: &[u8]) -> bool {
    k == [0x1b]
}

fn inverse_bar(text: &str, cols: usize) -> String {
    let w = cols.saturating_sub(1);
    let t: String = text.chars().take(w).collect();
    format!("\x1b[7m {t:<w$}\x1b[0m")
}

fn draw_bar(text: &str) {
    let (rows, cols) = term_size();
    out(format!("\x1b7\x1b[{rows};1H\x1b[2K{}\x1b8", inverse_bar(text, cols)).as_bytes());
}

// ── Text ──
// State files and logs are plain text on disk; scrub escape sequences before
// display so a tampered file can't drive the terminal.

fn strip_ansi(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            o.push(c);
            continue;
        }
        match it.next() {
            // CSI: parameters until a final byte
            Some('[') => {
                for d in it.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            // OSC: until BEL or ESC \
            Some(']') => {
                while let Some(d) = it.next() {
                    if d == '\x07' || (d == '\x1b' && it.next().is_some()) {
                        break;
                    }
                }
            }
            _ => {} // two-byte escape
        }
    }
    o
}

fn is_ctl(c: char) -> bool {
    c < ' ' || c == '\x7f'
}

// Metadata: control runs collapse to one space.
fn clean_meta(s: &str) -> String {
    let mut o = String::new();
    let mut prev_ctl = false;
    for c in strip_ansi(s).chars() {
        if is_ctl(c) {
            if !prev_ctl {
                o.push(' ');
            }
            prev_ctl = true;
        } else {
            o.push(c);
            prev_ctl = false;
        }
    }
    o.trim().to_string()
}

// Log text: drop escapes and control chars but keep tabs and line breaks.
fn clean_log(s: &str) -> String {
    strip_ansi(s).chars().filter(|&c| !is_ctl(c) || matches!(c, '\t' | '\n' | '\r')).collect()
}

fn js(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if c < ' ' => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn iso(ms: u64) -> String {
    // civil-from-days (Howard Hinnant)
    let secs = ms / 1000;
    let sod = secs % 86400;
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", sod / 3600, sod % 3600 / 60, sod % 60, ms % 1000)
}

fn format_uptime(start_ms: u64) -> String {
    let s = now_ms().saturating_sub(start_ms) / 1000;
    match s {
        0..=59 => format!("up {s}s"),
        60..=3599 => format!("up {}m{:02}s", s / 60, s % 60),
        _ => format!("up {}h{:02}m", s / 3600, s % 3600 / 60),
    }
}

// ── Files ──

fn file_len(p: &Path) -> Option<u64> {
    fs::metadata(p).ok().map(|m| m.len())
}

fn read_range(p: &Path, start: u64, end: u64) -> Vec<u8> {
    let mut b = Vec::new();
    if let Ok(mut f) = File::open(p) {
        if f.seek(SeekFrom::Start(start)).is_ok() {
            let _ = f.take(end.saturating_sub(start)).read_to_end(&mut b);
        }
    }
    b
}

// Last `max` bytes before `end`, minus the partial first line when cut.
fn read_tail(p: &Path, end: u64, max: u64) -> String {
    let start = end.saturating_sub(max);
    let t = String::from_utf8_lossy(&read_range(p, start, end)).into_owned();
    match t.find('\n') {
        Some(i) if start > 0 => t[i + 1..].to_string(),
        _ => t,
    }
}

fn read_log_tail(p: &Path, max: u64) -> String {
    file_len(p).map_or_else(String::new, |len| read_tail(p, len, max))
}

fn append(p: &Path, text: &str) {
    if let Ok(mut f) = OpenOptions::new().append(true).create(true).mode(0o600).open(p) {
        let _ = f.write_all(text.as_bytes());
    }
}

fn truncate_log(p: &Path) {
    let Some(size) = file_len(p) else { return };
    if size <= MAX_LOG_BYTES {
        return;
    }
    let b = read_range(p, size - MAX_LOG_BYTES / 2, size);
    let kept = b.iter().position(|&c| c == b'\n').map_or(&b[..], |i| &b[i + 1..]);
    // Truncate then append, never rewrite from offset 0: a live O_APPEND
    // writer's lines land before or after ours instead of inside them.
    // ponytail: lines written between the read and the truncate are lost.
    let Ok(mut f) = OpenOptions::new().append(true).open(p) else { return };
    if f.set_len(0).is_ok() {
        let mut data = format!("[whiskd] log truncated (exceeded {}MB)\n", MAX_LOG_BYTES / 1024 / 1024).into_bytes();
        data.extend_from_slice(kept);
        let _ = f.write_all(&data);
    }
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn list_dirs(d: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(d) else { return vec![] };
    rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect()
}

fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

// ── Process state ──

#[derive(Clone)]
struct Proc {
    name: String,
    dir: PathBuf,
    cwd: Option<String>,
}

fn proc_dir(name: &str) -> PathBuf {
    ctx().dir.join(name)
}
fn log_file(dir: &Path) -> PathBuf {
    dir.join("output.log")
}
fn pid_file(dir: &Path) -> PathBuf {
    dir.join("pid")
}
fn cmd_file(dir: &Path) -> PathBuf {
    dir.join("cmd")
}
fn started_file(dir: &Path) -> PathBuf {
    dir.join("started")
}
fn cwd_file(dir: &Path) -> PathBuf {
    dir.join("cwd")
}

fn pid_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn get_pid(dir: &Path) -> Option<i32> {
    let pid: i32 = read_trim(&pid_file(dir))?.parse().ok()?;
    (pid > 1 && pid_alive(pid)).then_some(pid) // <=1: garbage, process groups, or init
}

fn get_cmd(dir: &Path) -> String {
    fs::read_to_string(cmd_file(dir)).map_or_else(|_| "?".into(), |s| clean_meta(&s))
}

fn get_started(dir: &Path) -> Option<u64> {
    read_trim(&started_file(dir))?.parse().ok()
}

fn write_state(dir: &Path, pid: u32, cmd: &str, started: u64) {
    let files = [
        (pid_file(dir), pid.to_string()),
        (cmd_file(dir), cmd.to_string()),
        (started_file(dir), started.to_string()),
        (cwd_file(dir), ctx().cwd.to_string_lossy().into_owned()),
    ];
    for (p, v) in files {
        let res = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&p).and_then(|mut f| f.write_all(v.as_bytes()));
        if let Err(e) = res {
            die(&format!("cannot write process state in {}: {e}", dir.display()));
        }
    }
}

fn procs_in(d: &Path) -> Vec<Proc> {
    let mut v: Vec<Proc> = list_dirs(d)
        .into_iter()
        .filter(|p| cmd_file(p).exists())
        .map(|dir| Proc { name: file_name(&dir), dir, cwd: None })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn all_procs() -> Vec<Proc> {
    procs_in(&ctx().dir)
}

fn all_procs_global() -> Vec<Proc> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut v: Vec<Proc> = list_dirs(&ctx().base).iter().flat_map(|d| procs_in(d)).collect();
    for p in &mut v {
        let cwd = fs::read_to_string(cwd_file(&p.dir)).map_or_else(|_| "?".into(), |s| clean_meta(&s));
        let tilde = !home.is_empty() && (cwd == home || cwd.starts_with(&format!("{home}/")));
        p.cwd = Some(if tilde { format!("~{}", &cwd[home.len()..]) } else { cwd });
    }
    v
}

// ── PID identity ──
// A pid file can outlive its process and the OS can recycle the pid. Before
// acting on a "live" pid, compare its start time from `ps` with the one we
// recorded; >60s apart means it isn't ours. Display paths skip this.

fn parse_etime(s: &str) -> Option<u64> {
    // ps etime formats: mm:ss / hh:mm:ss / dd-hh:mm:ss
    let (days, rest) = match s.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, s),
    };
    let parts: Vec<u64> = rest.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let (h, m, sec) = match parts[..] {
        [m, s] => (0, m, s),
        [h, m, s] => (h, m, s),
        _ => return None,
    };
    Some(((days * 24 + h) * 60 + m) * 60 + sec)
}

fn pid_identity_ok(dir: &Path, pid: i32) -> bool {
    let Some(recorded) = get_started(dir) else { return true }; // nothing recorded → trust
    // ponytail: one ps spawn per check; batch with `ps -ax` if many procs make this slow
    let Ok(o) = Command::new("ps").args(["-o", "etime=", "-p", &pid.to_string()]).output() else { return true };
    let Some(sec) = parse_etime(String::from_utf8_lossy(&o.stdout).trim()) else { return true }; // died since → harmless
    now_ms().saturating_sub(sec * 1000).abs_diff(recorded) < 60_000
}

// ── Killing ──
// Claim-first: whoever renames the pid file away owns the exit footer, so
// concurrent whiskd invocations never write duplicates. rename(2), unlike
// unlink(2) on macOS, hands the entry to exactly one claimer.

fn mark_dead(name: &str, dir: &Path, footer: &str) -> bool {
    let claim = dir.join("pid.reaped");
    if fs::rename(pid_file(dir), &claim).is_err() {
        return false;
    }
    let log = log_file(dir);
    append(&log, &format!("\n[whiskd:{name}] {footer}\n"));
    truncate_log(&log);
    let _ = fs::remove_file(claim);
    true
}

fn kill_tree(pid: i32, sig: i32) {
    unsafe {
        libc::kill(-pid, sig);
        libc::kill(pid, sig);
    }
}

fn exit_desc(st: ExitStatus) -> String {
    match (st.code(), st.signal()) {
        (Some(c), _) => format!("exited with code {c}"),
        (None, Some(s)) => format!("killed by {}", sig_name(s)),
        _ => "exited".into(),
    }
}

fn exit_code(st: ExitStatus) -> i32 {
    st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0))
}

fn sig_name(s: i32) -> String {
    match s {
        1 => "SIGHUP".into(),
        2 => "SIGINT".into(),
        3 => "SIGQUIT".into(),
        6 => "SIGABRT".into(),
        9 => "SIGKILL".into(),
        13 => "SIGPIPE".into(),
        15 => "SIGTERM".into(),
        _ => format!("signal {s}"),
    }
}

struct Target {
    name: String,
    dir: PathBuf,
    pid: i32,
}

// SIGTERM the tree, SIGKILL survivors at 2s, give up at 3s (uninterruptible
// sleep). tick() is non-blocking so top can drive it from its loop.
struct Reaper {
    silent: bool,
    items: Vec<(Target, Instant, bool)>,
}

impl Reaper {
    fn new(silent: bool) -> Self {
        Reaper { silent, items: vec![] }
    }

    fn add(&mut self, t: Target) {
        kill_tree(t.pid, libc::SIGTERM);
        self.items.push((t, Instant::now(), false));
    }

    fn tick(&mut self) -> bool {
        let silent = self.silent;
        self.items.retain_mut(|(t, since, escalated)| {
            let el = since.elapsed();
            if !pid_alive(t.pid) || el >= Duration::from_secs(3) {
                if pid_alive(t.pid) && !silent {
                    println!("warning: {} (pid {}) did not die", t.name, t.pid);
                }
                mark_dead(&t.name, &t.dir, "stopped");
                return false;
            }
            if !*escalated && el >= Duration::from_secs(2) {
                *escalated = true;
                kill_tree(t.pid, libc::SIGKILL);
                if !silent {
                    println!("force killed {}", t.name);
                }
            }
            true
        });
        self.items.is_empty()
    }

    fn finish(&mut self) {
        while !self.tick() {
            sleep(TICK);
        }
    }
}

// ── Housekeeping ──
// Runs before every command. The parent that would write the exit footer
// usually exits long before its detached child, so deaths are discovered
// here: dead or recycled pid → clear the pid file and record the exit.

fn ensure_base() {
    let base = &ctx().base;
    let _ = fs::DirBuilder::new().recursive(true).mode(0o700).create(base);
    // Create-then-verify with lstat so a planted symlink is always caught.
    let st = fs::symlink_metadata(base).unwrap_or_else(|e| die(&format!("cannot create state dir {}: {e}", base.display())));
    if st.file_type().is_symlink() || !st.is_dir() {
        die(&format!("state path {0} is not a directory (possible tampering).\n  remove it with: rm -rf {0}", base.display()));
    }
    let uid = unsafe { libc::getuid() };
    if st.uid() != uid {
        die(&format!("state dir {0} is owned by uid {1}, not you (uid {uid}).\n  remove it with: rm -rf {0}", base.display(), st.uid()));
    }
    let _ = fs::set_permissions(base, fs::Permissions::from_mode(0o700));
}

fn reconcile(dir: &Path) {
    let Some(raw) = read_trim(&pid_file(dir)) else { return };
    if let Ok(pid) = raw.parse::<i32>() {
        if pid > 1 && pid_alive(pid) && pid_identity_ok(dir, pid) {
            truncate_log(&log_file(dir)); // cap is safe under O_APPEND writers
            return;
        }
    }
    mark_dead(&file_name(dir), dir, "exited (status unknown)");
}

fn housekeep() {
    ensure_base();
    // Dirs without a cmd file (e.g. the pre-4.0 nested layout) are left alone.
    for cwd_dir in list_dirs(&ctx().base) {
        for p in list_dirs(&cwd_dir) {
            if !cmd_file(&p).exists() {
                continue;
            }
            reconcile(&p);
            let old = fs::metadata(cmd_file(&p))
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|e| e.as_millis() > MAX_AGE_MS));
            if old && !pid_file(&p).exists() {
                let _ = fs::remove_dir_all(&p);
            }
        }
        let _ = fs::remove_dir(&cwd_dir); // only succeeds when empty
    }
}

// ── Naming ──

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '-' })
        .take(30)
        .collect()
}

fn is_ident(k: &str) -> bool {
    let mut cs = k.chars();
    cs.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_env_assign(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| is_ident(k))
}

// npm/yarn/pnpm [run] <x> and bun run <x> → script; node/python/... [flags]
// <file> → file name; otherwise the first word that isn't a flag or env
// assignment. `go run .` and friends fall back to the directory name.
fn derive_name(args: &[String]) -> String {
    let words: Vec<&str> = args.iter().flat_map(|a| a.split_whitespace()).collect();
    let operand = |i: usize| words.get(i..).and_then(|r| r.iter().find(|w| !w.starts_with('-'))).copied();
    let mut pick = None;
    for (i, w) in words.iter().enumerate() {
        let run = words.get(i + 1) == Some(&"run");
        pick = match file_name(Path::new(w)).as_str() {
            "npm" | "yarn" | "pnpm" => operand(i + 1 + run as usize),
            "bun" | "deno" | "go" if run => operand(i + 2),
            "node" | "python" | "python3" | "ruby" | "bun" | "deno" => operand(i + 1),
            _ => None,
        };
        if pick.is_some() {
            break;
        }
    }
    let pick = pick.or_else(|| words.iter().find(|w| w.len() > 1 && !w.starts_with('-') && !is_env_assign(w)).copied());
    let stem = pick.and_then(|p| Path::new(p).file_stem()).map(|s| sanitize(&s.to_string_lossy())).unwrap_or_default();
    if stem.chars().any(|c| c.is_ascii_alphanumeric()) {
        return stem;
    }
    let dir = sanitize(&file_name(&ctx().cwd));
    if dir.is_empty() { format!("proc-{:04x}", now_ms() & 0xffff) } else { dir }
}

fn unique_name(base: &str) -> String {
    if get_pid(&proc_dir(base)).is_none() {
        return base.into();
    }
    (2..=99)
        .map(|i| format!("{base}-{i}"))
        .find(|n| get_pid(&proc_dir(n)).is_none())
        .unwrap_or_else(|| format!("{base}-{:04x}", now_ms() & 0xffff))
}

// Explicit names are a contract (agents reuse them for logs/stop), so a
// clash is an error rather than a silent `-2` suffix.
fn pick_name(explicit: Option<String>, args: &[String]) -> String {
    let Some(name) = explicit else { return unique_name(&derive_name(args)) };
    if let Some(pid) = get_pid(&proc_dir(&name)) {
        die(&format!("{name} is already running (pid {pid}). stop it first: whiskd stop {name}"));
    }
    name
}

// Only a leading flag counts: a later -n belongs to the command (tail -n 3).
fn parse_name_flag(args: &[String]) -> (Option<String>, Vec<String>) {
    match args {
        [f, n, rest @ ..] if (f == "--name" || f == "-n") && !n.is_empty() => (Some(sanitize(n)), rest.to_vec()),
        _ => (None, args.to_vec()),
    }
}

fn resolve_name(explicit: Option<&str>) -> Option<Proc> {
    if let Some(e) = explicit {
        // Stored names are sanitized at creation; sanitize the lookup too.
        let name = sanitize(e);
        return Some(Proc { dir: proc_dir(&name), name, cwd: None });
    }
    let procs = all_procs();
    let running: Vec<&Proc> = procs.iter().filter(|p| get_pid(&p.dir).is_some()).collect();
    match (running.len(), procs.len()) {
        (1, _) => Some(running[0].clone()),
        (0, 0) => None,
        (0, 1) => Some(procs[0].clone()),
        (0, _) => {
            eprintln!("multiple processes found. specify a name:");
            procs.iter().for_each(|p| eprintln!("  {}", p.name));
            exit(1)
        }
        _ => {
            eprintln!("multiple running processes. specify a name:");
            running.iter().for_each(|p| eprintln!("  {}  (pid {})", p.name, get_pid(&p.dir).unwrap_or(0)));
            exit(1)
        }
    }
}

// ── Shell quoting ──
// A single arg is passed raw so `whiskd start "a && b"` keeps its shell
// semantics. Multiple args are quoted per-arg so the grouping the user's
// shell already resolved survives the inner shell.

fn quote_one(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn quote_arg(a: &str) -> String {
    if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:=@%+".contains(c)) {
        return a.into();
    }
    // FOO=a b → FOO='a b': keep the assignment outside the quotes
    match a.split_once('=') {
        Some((k, v)) if is_ident(k) => format!("{k}={}", quote_one(v)),
        _ => quote_one(a),
    }
}

fn shell_quote(args: &[String]) -> String {
    if args.len() == 1 {
        return args[0].clone();
    }
    args.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ")
}

// ── Spawning ──
// The child runs in its own session and owns the log fd, so whiskd exiting,
// detaching, or dying never kills or silences it.

fn spawn_managed(name: &str, cmd: &str, force_color: &str) -> (PathBuf, Child) {
    let dir = proc_dir(name);
    if let Err(e) = fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir) {
        die(&format!("cannot create process dir {}: {e}", dir.display()));
    }
    let log = log_file(&dir);
    // Fresh log per run. Unlink first so a previous writer keeps its own
    // inode; O_APPEND so truncation under a live writer can't leave a hole.
    let _ = fs::remove_file(&log);
    let mut f = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(&log)
        .unwrap_or_else(|e| die(&format!("cannot open log {}: {e}", log.display())));
    let now = now_ms();
    let _ = write!(f, "[whiskd:{name}] started: {cmd}\n[whiskd:{name}] time: {}\n[whiskd:{name}] {}\n", iso(now), "—".repeat(60));
    let err = f.try_clone().unwrap_or_else(|e| die(&format!("cannot open log {}: {e}", log.display())));
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(cmd).stdin(Stdio::null()).stdout(f).stderr(err).env("FORCE_COLOR", force_color);
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    match c.spawn() {
        Ok(child) => {
            write_state(&dir, child.id(), cmd, now);
            (dir, child)
        }
        Err(e) => {
            append(&log, &format!("\n[whiskd:{name}] failed to start: {e}\n"));
            die(&format!("[{name}] failed to start: {e}"))
        }
    }
}

// ── Attach ──

enum End {
    Exited,
    Detached,
    Killed,
}

struct Session {
    log: PathBuf,
    pos: u64,
    paused: bool,
    held: Vec<u8>,
}

impl Session {
    fn pull(&mut self) -> Vec<u8> {
        let Some(size) = file_len(&self.log) else { return vec![] };
        self.pos = self.pos.min(size); // log truncated under us
        let b = read_range(&self.log, self.pos, size);
        self.pos += b.len() as u64;
        b
    }

    fn hold(&mut self, b: &[u8]) {
        self.held.extend_from_slice(b);
        if self.held.len() > MAX_PAUSE_BYTES {
            self.held.drain(..self.held.len() - MAX_PAUSE_BYTES);
        }
    }

    // Final catch-up so footers and last output are never lost.
    fn drain(&mut self) {
        let mut b = std::mem::take(&mut self.held);
        b.extend(self.pull());
        out(&b);
    }
}

// With `child` (foreground mode) the caller owns the process: kill keys
// signal it here (second press escalates) and death comes from try_wait with
// the real status. Without it, death is detected by polling the pid file and
// a kill key returns Killed for the caller to act on.
fn run_attach(p: &Proc, safe_quit: bool, mut child: Option<&mut Child>) -> (End, Session) {
    let log = log_file(&p.dir);
    let mut s = Session { log: log.clone(), pos: 0, paused: false, held: vec![] };
    let Some(pid) = get_pid(&p.dir) else { return (End::Exited, s) };
    let Some(size) = file_len(&log) else { return (End::Detached, s) };
    catch_signals();

    let mut size_now = term_size();
    setup_scroll_region(size_now.0);
    raw_on();
    let tail = read_tail(&log, size, LOG_TAIL_BYTES);
    let lines: Vec<&str> = tail.split('\n').collect();
    let keep = lines.len().saturating_sub(size_now.0.saturating_sub(5));
    out(lines[keep..].iter().map(|l| format!("{l}\n")).collect::<String>().as_bytes());
    s.pos = size;

    let cmd = get_cmd(&p.dir);
    let started = get_started(&p.dir);
    let keys = if safe_quit { "q/Esc: back  Ctrl+C: kill  d: detach  p: pause" } else { "q/Ctrl+C: quit+kill  d: detach  p: pause" };
    let bar = |paused: bool| {
        let up = started.map_or_else(|| "up ?".into(), format_uptime);
        format!("{}  │  {cmd}  │  pid {pid}  │  {up}{}  │  {keys}", p.name, if paused { "  [PAUSED]" } else { "" })
    };
    draw_bar(&bar(false));

    let mut last_sec = Instant::now();
    let mut killing: Option<Instant> = None;
    let mut escalated = false;
    let end = loop {
        check_signal();
        if let Some(k) = read_key(100) {
            if safe_quit && (k[0] == b'q' || is_esc(&k)) {
                break End::Detached;
            }
            if k[0] == 0x03 || (!safe_quit && k[0] == b'q') {
                let Some(c) = child.as_deref_mut() else { break End::Killed };
                if killing.is_none() {
                    kill_tree(c.id() as i32, libc::SIGTERM);
                    killing = Some(Instant::now());
                } else {
                    kill_tree(c.id() as i32, libc::SIGKILL);
                    escalated = true;
                }
            } else if k[0] == b'd' {
                break End::Detached;
            } else if k[0] == b'p' {
                s.paused = !s.paused;
                if !s.paused {
                    out(&std::mem::take(&mut s.held));
                }
                draw_bar(&bar(s.paused));
            }
        }
        let data = s.pull();
        if !data.is_empty() {
            if s.paused { s.hold(&data) } else { out(&data) }
            draw_bar(&bar(s.paused));
        }
        if let Some(c) = child.as_deref_mut() {
            if matches!(c.try_wait(), Ok(Some(_))) {
                break if killing.is_some() { End::Killed } else { End::Exited };
            }
            if !escalated && killing.is_some_and(|t| t.elapsed() >= Duration::from_secs(2)) {
                kill_tree(c.id() as i32, libc::SIGKILL);
                escalated = true;
            }
        }
        if term_size() != size_now {
            size_now = term_size();
            setup_scroll_region(size_now.0);
            draw_bar(&bar(s.paused));
        }
        if last_sec.elapsed() >= Duration::from_secs(1) {
            last_sec = Instant::now();
            truncate_log(&log); // keep the cap enforced during long sessions
            draw_bar(&bar(s.paused));
            if child.is_none() && get_pid(&p.dir).is_none() {
                // Nobody else recorded this death (or a concurrent stop did,
                // in which case mark_dead loses the claim and writes nothing).
                mark_dead(&p.name, &p.dir, "exited (status unknown)");
                break End::Exited;
            }
        }
    };
    reset_scroll_region();
    raw_off();
    (end, s)
}

fn kill_attached(p: &Proc, pid: i32, s: &mut Session) -> ! {
    if !pid_identity_ok(&p.dir, pid) {
        mark_dead(&p.name, &p.dir, "exited (status unknown)");
        s.drain();
        println!("\n[whiskd:{}] already exited (stale pid)", p.name);
        exit(0);
    }
    let mut r = Reaper::new(false);
    r.add(Target { name: p.name.clone(), dir: p.dir.clone(), pid });
    r.finish();
    s.drain();
    println!("\n[whiskd:{}] killed", p.name);
    exit(0)
}

// ── Top ──

const COL_NAME_W: usize = 16;
const COL_DIR_W: usize = 22;
const COL_PID_W: usize = 8;
const COL_UPTIME_W: usize = 10;

fn trunc_pad(s: &str, w: usize, right: bool) -> String {
    if s.chars().count() > w {
        let mut t: String = s.chars().take(w - 1).collect();
        t.push('~');
        t
    } else if right {
        format!("{s:>w$}")
    } else {
        format!("{s:<w$}")
    }
}

fn format_columns(name: &str, pid: &str, uptime: &str, cmd: &str, cols: usize, dir: Option<&str>) -> String {
    let fixed = COL_NAME_W + dir.map_or(0, |_| COL_DIR_W + 1) + COL_PID_W + COL_UPTIME_W + 4;
    let mut parts = vec![trunc_pad(name, COL_NAME_W, false)];
    if let Some(d) = dir {
        parts.push(trunc_pad(d, COL_DIR_W, false));
    }
    parts.push(trunc_pad(pid, COL_PID_W, true));
    parts.push(trunc_pad(uptime, COL_UPTIME_W, true));
    format!("{}  {}", parts.join(" "), trunc_pad(cmd, cols.saturating_sub(fixed).max(10), false))
}

#[derive(Clone)]
struct Row {
    name: String,
    dir: PathBuf,
    cwd: String,
    pid: i32,
    uptime: String,
    cmd: String,
}

struct LogView {
    name: String,
    lines: Vec<String>,
    scroll: usize,
}

struct Top {
    global: bool,
    cursor: usize,
    list: Vec<Row>,
    logs: Option<LogView>,
    reaper: Reaper,
}

impl Top {
    // Re-read everything each tick: a name can be reused by a new process
    // between ticks, and N is small enough that caching isn't worth staleness.
    fn refresh(&mut self) {
        let prev = self.list.get(self.cursor).map(|r| r.dir.clone());
        let procs = if self.global { all_procs_global() } else { all_procs() };
        self.list = procs
            .into_iter()
            .filter_map(|p| {
                Some(Row {
                    pid: get_pid(&p.dir)?,
                    uptime: get_started(&p.dir).map_or_else(|| "---".into(), format_uptime),
                    cmd: get_cmd(&p.dir),
                    cwd: p.cwd.unwrap_or_else(|| ".".into()),
                    name: p.name,
                    dir: p.dir,
                })
            })
            .collect();
        self.list.sort_by(|a, b| (&a.name, &a.cwd).cmp(&(&b.name, &b.cwd)));
        if let Some(i) = prev.and_then(|d| self.list.iter().position(|r| r.dir == d)) {
            self.cursor = i;
        }
        self.cursor = self.cursor.min(self.list.len().saturating_sub(1));
    }

    fn render(&self) {
        let (rows, cols) = term_size();
        let mut o = String::from("\x1b[H");
        if let Some(lv) = &self.logs {
            let avail = rows.saturating_sub(3);
            o += &format!("\x1b[2K\x1b[1mwhiskd logs -- {}\x1b[0m\n\x1b[2K\n", lv.name);
            let start = lv.lines.len().saturating_sub(avail + lv.scroll);
            for i in 0..avail {
                o += "\x1b[2K";
                if let Some(l) = lv.lines.get(start + i) {
                    o += trunc_pad(l, cols, false).trim_end();
                }
                o += "\n";
            }
            o += &format!("\x1b[{rows};1H\x1b[2K{}", inverse_bar("q/Esc: back  ↑/↓/j/k: scroll", cols));
            return out(o.as_bytes());
        }
        let n = self.list.len();
        let dir_col = |d| self.global.then_some(d);
        o += &format!("\x1b[2Kwhiskd top{} -- {n} running process{}\n", if self.global { " (global)" } else { "" }, if n == 1 { "" } else { "es" });
        o += &format!("\x1b[2K\x1b[1m{}\x1b[0m\n\x1b[2K\n", format_columns("NAME", "PID", "UPTIME", "COMMAND", cols, dir_col("DIR")));
        let avail = rows.saturating_sub(4);
        for i in 0..avail {
            o += "\x1b[2K";
            if n == 0 && i == avail.saturating_sub(1) / 2 {
                let msg = "no running processes -- whiskd start <cmd> to begin";
                o += &format!("\x1b[2m{msg:>w$}\x1b[0m", w = (cols + msg.len()) / 2);
            } else if let Some(r) = self.list.get(i) {
                let label = format!("{}{}", if i == self.cursor { "▸ " } else { "  " }, r.name);
                let line = format_columns(&label, &r.pid.to_string(), &r.uptime, &r.cmd, cols, dir_col(&r.cwd));
                o += &if i == self.cursor { format!("\x1b[7m{line}\x1b[0m") } else { line };
            }
            o += "\n";
        }
        let hints = format!("q: quit  ↑/↓/j/k: select  Enter/a: attach  s: stop  l: logs  g: {}", if self.global { "local" } else { "global" });
        o += &format!("\x1b[{rows};1H\x1b[2K{}", inverse_bar(&hints, cols));
        out(o.as_bytes());
    }

    fn key(&mut self, k: &[u8]) {
        if let Some(lv) = self.logs.as_mut() {
            if k[0] == b'q' || is_esc(k) || k[0] == 0x7f {
                self.logs = None;
            } else if is_up(k) {
                let max = lv.lines.len().saturating_sub(term_size().0.saturating_sub(3));
                lv.scroll = (lv.scroll + 1).min(max);
            } else if is_down(k) {
                lv.scroll = lv.scroll.saturating_sub(1);
            }
            return self.render();
        }
        let selected = self.list.get(self.cursor).cloned();
        match k[0] {
            b'q' | 0x03 => {
                // Finish pending stops so escalation isn't lost on quit.
                leave_alt();
                self.reaper.finish();
                exit(0)
            }
            _ if is_up(k) => self.cursor = self.cursor.saturating_sub(1),
            _ if is_down(k) => self.cursor = (self.cursor + 1).min(self.list.len().saturating_sub(1)),
            0x0d | b'a' => {
                if let Some(r) = selected {
                    leave_alt();
                    let p = Proc { name: r.name, dir: r.dir, cwd: None };
                    let (end, mut s) = run_attach(&p, true, None);
                    if let End::Killed = end {
                        kill_attached(&p, r.pid, &mut s);
                    }
                    enter_alt();
                    self.refresh();
                }
            }
            b's' => {
                if let Some(r) = selected {
                    if pid_identity_ok(&r.dir, r.pid) {
                        self.reaper.add(Target { name: r.name, dir: r.dir, pid: r.pid });
                    } else {
                        mark_dead(&r.name, &r.dir, "exited (status unknown)");
                    }
                    self.refresh();
                }
            }
            b'l' => {
                if let Some(r) = selected {
                    // Strip at load, not render: render slices lines by width.
                    // After a \r only the last overwrite is what a terminal shows.
                    let text = clean_log(&read_log_tail(&log_file(&r.dir), 1024 * 1024));
                    let mut lines: Vec<String> = text
                        .trim_end_matches('\n')
                        .split('\n')
                        .map(|l| l.trim_end_matches('\r').rsplit('\r').next().unwrap_or("").to_string())
                        .collect();
                    if text.is_empty() {
                        lines = vec!["(no logs)".into()];
                    }
                    self.logs = Some(LogView { name: r.name, lines, scroll: 0 });
                }
            }
            b'g' => {
                self.global = !self.global;
                self.cursor = 0;
                self.refresh();
            }
            _ => {}
        }
        self.render();
    }
}

// ── Commands ──

fn cmd_status(json: bool) {
    let procs = all_procs();
    if json {
        let cwd = ctx().cwd.to_string_lossy();
        let items: Vec<String> = procs
            .iter()
            .map(|p| {
                let pid = get_pid(&p.dir);
                let started = get_started(&p.dir);
                let fields = [
                    ("name", js(&p.name)),
                    ("cwd", js(&cwd)),
                    ("status", js(if pid.is_some() { "running" } else { "stopped" })),
                    ("pid", pid.map_or_else(|| "null".into(), |p| p.to_string())),
                    ("started", started.map_or_else(|| "null".into(), |s| js(&iso(s)))),
                    ("cmd", js(&get_cmd(&p.dir))),
                    ("log", js(&log_file(&p.dir).to_string_lossy())),
                    ("uptime", pid.and(started).map_or_else(|| "null".into(), |s| js(&format_uptime(s)))),
                ];
                let body: Vec<String> = fields.iter().map(|(k, v)| format!("    \"{k}\": {v}")).collect();
                format!("  {{\n{}\n  }}", body.join(",\n"))
            })
            .collect();
        if items.is_empty() { println!("[]") } else { println!("[\n{}\n]", items.join(",\n")) }
        return;
    }
    if procs.is_empty() {
        return println!("no processes");
    }
    for p in procs {
        let pid = get_pid(&p.dir);
        println!(
            "process  name={}  status={}  pid={}  started={}  cmd=\"{}\"  log={}",
            p.name,
            if pid.is_some() { "running" } else { "stopped" },
            pid.map_or_else(|| "—".into(), |p| p.to_string()),
            get_started(&p.dir).map_or_else(|| "?".into(), iso),
            get_cmd(&p.dir),
            log_file(&p.dir).display()
        );
    }
}

fn cmd_stop(arg: Option<&str>) {
    let mut reaper = Reaper::new(false);
    let targets: Vec<Proc> = if arg == Some("--all") {
        all_procs()
    } else {
        match resolve_name(arg) {
            Some(p) => vec![p],
            None => return println!("nothing running"),
        }
    };
    let single = arg != Some("--all");
    for p in targets {
        let Some(pid) = get_pid(&p.dir) else {
            if single {
                println!("{}: not running", p.name);
            }
            continue;
        };
        if !pid_identity_ok(&p.dir, pid) {
            mark_dead(&p.name, &p.dir, "exited (status unknown)");
            println!("{}: not running (pid {pid} belongs to another process — cleaned up stale state)", p.name);
            continue;
        }
        println!("stopped  {}  pid={pid}", p.name);
        reaper.add(Target { name: p.name, dir: p.dir, pid });
    }
    if !single && reaper.items.is_empty() {
        println!("nothing running");
    }
    reaper.finish();
}

fn cmd_clean() {
    let mut cleaned = 0;
    for p in all_procs() {
        if get_pid(&p.dir).is_none() {
            let _ = fs::remove_dir_all(&p.dir);
            println!("removed  {}", p.name);
            cleaned += 1;
        }
    }
    if cleaned == 0 {
        println!("nothing to clean");
    }
    let _ = fs::remove_dir(&ctx().dir); // only succeeds when empty
}

fn cmd_logs(args: &[String]) {
    let mut name = None;
    let mut n = 40;
    for a in args {
        // A number is a line count unless a process has that name.
        match a.parse::<usize>() {
            Ok(v) if !cmd_file(&proc_dir(a)).exists() => n = v,
            _ => name = Some(a.as_str()),
        }
    }
    let Some(p) = resolve_name(name) else { return println!("no processes") };
    let log = log_file(&p.dir);
    if !log.exists() {
        return println!("{}: no logs yet", p.name);
    }
    let text = clean_log(&read_log_tail(&log, LOG_TAIL_BYTES));
    let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    println!("{}", lines[lines.len().saturating_sub(n)..].join("\n"));
}

fn cmd_attach(arg: Option<&str>) {
    if !is_tty(0) {
        eprintln!("whiskd attach requires a terminal (use whiskd logs)");
        exit(1);
    }
    let Some(p) = resolve_name(arg) else { return println!("nothing to attach to") };
    let Some(pid) = get_pid(&p.dir) else { return println!("{}: not running", p.name) };
    let (end, mut s) = run_attach(&p, false, None);
    match end {
        End::Killed => kill_attached(&p, pid, &mut s),
        End::Exited => {
            s.drain();
            println!("\n[whiskd:{}] process exited", p.name)
        }
        End::Detached => {
            s.drain();
            println!("\n[whiskd:{}] detached. still running.", p.name)
        }
    }
}

fn cmd_top(global: bool) {
    if !is_tty(0) {
        eprintln!("whiskd top requires a terminal");
        exit(1);
    }
    catch_signals();
    let mut t = Top { global, cursor: 0, list: vec![], logs: None, reaper: Reaper::new(true) };
    enter_alt();
    t.refresh();
    t.render();
    let mut last = Instant::now();
    let mut size = term_size();
    loop {
        if SIGNAL.load(SeqCst) != 0 {
            leave_alt();
            t.reaper.finish();
            check_signal();
        }
        let key = read_key(100);
        t.reaper.tick();
        if let Some(k) = key {
            t.key(&k);
        }
        if last.elapsed() >= Duration::from_secs(1) {
            last = Instant::now();
            t.refresh();
            t.list.iter().for_each(|r| truncate_log(&log_file(&r.dir)));
            if t.logs.is_none() {
                t.render();
            }
        }
        if term_size() != size {
            size = term_size();
            t.render();
        }
    }
}

fn next_steps(name: &str) {
    println!("\n  whiskd attach {name}\n  whiskd logs {name}\n  whiskd stop {name}");
}

fn cmd_start(args: &[String]) {
    let (explicit, rest) = parse_name_flag(args);
    if rest.is_empty() {
        eprintln!("usage: whiskd start [--name <n>] <command...>");
        exit(1);
    }
    let name = pick_name(explicit, &rest);
    let (dir, mut child) = spawn_managed(&name, &shell_quote(&rest), "0");
    println!("started  name={name}  pid={}", child.id());
    println!("logs     {}", log_file(&dir).display());
    next_steps(&name);
    // Surface boot crashes so callers don't mistake them for a start. Later
    // deaths are discovered by housekeeping.
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(300) {
        if let Ok(Some(st)) = child.try_wait() {
            let d = exit_desc(st);
            mark_dead(&name, &dir, &d);
            eprintln!("\n[whiskd:{name}] {d} right after starting. last output:");
            let text = clean_log(&read_log_tail(&log_file(&dir), LOG_TAIL_BYTES));
            let lines: Vec<&str> = text.trim_end().split('\n').collect();
            eprintln!("{}", lines[lines.len().saturating_sub(10)..].join("\n"));
            exit(exit_code(st));
        }
        sleep(Duration::from_millis(20));
    }
}

// Foreground spawns exactly like start, then attaches to its own process, so
// detaching is only UI teardown. FORCE_COLOR=1 here because it's a terminal
// view; start uses 0.
fn cmd_fg(args: &[String]) {
    let (explicit, rest) = parse_name_flag(args);
    if rest.is_empty() {
        eprintln!("usage: whiskd [--name <name>] <command...>");
        exit(1);
    }
    let name = pick_name(explicit, &rest);
    let (dir, mut child) = spawn_managed(&name, &shell_quote(&rest), "1");
    let p = Proc { name: name.clone(), dir: dir.clone(), cwd: None };
    let (end, mut s) = run_attach(&p, false, Some(&mut child));
    if let End::Detached = end {
        s.drain();
        println!("\n[whiskd:{name}] detached. still running.");
        next_steps(&name);
        exit(0);
    }
    let st = child.wait().unwrap_or_else(|e| die(&format!("wait failed: {e}")));
    // Real exit footer; skipped if a concurrent stop already claimed it.
    mark_dead(&name, &dir, &exit_desc(st));
    s.drain();
    if let End::Killed = end {
        println!("\n[whiskd:{name}] killed");
        exit(1);
    }
    println!("\n[whiskd:{name}] {}", exit_desc(st));
    exit(exit_code(st));
}

fn help(base: &Path) {
    println!(
        "whiskd {VERSION} - lightweight process wrapper with TUI

  whiskd <cmd...>                  run with TUI, auto-named
  whiskd start <cmd...>            run in background, auto-named
  whiskd start --name api <cmd>    run in background with explicit name
                                   (--name/-n must come before the command)
  whiskd status [--json]           list processes (--json for machine-readable)
  whiskd top [--global]            live running-process dashboard (--global/-g for all dirs)
  whiskd logs [name] [n]           last n lines (default 40)
  whiskd attach [name]             attach TUI to running process
  whiskd stop [name]               stop a process (or all with --all)
  whiskd clean                     remove stopped process dirs

TUI keys (foreground/attach):
  q/Ctrl+C  kill process and exit (default)
  d         detach (leave process running in background)
  p         pause/unpause output

Top keys:
  q/Ctrl+C  quit top
  ↑/↓/j/k   select process
  Enter/a   attach to selected process
  s         stop selected process
  l         view logs of selected process
  g         toggle global/local view

Names are auto-derived from commands:
  npm run dev → dev    node server.js → server
  bun run build → build    python app.py → app

State and logs live in {}/<cwd-hash>/<name>/output.log",
        base.display()
    );
}

fn main() {
    // Die quietly on a closed pipe (`whiskd logs | head`) instead of panicking.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    std::panic::set_hook(Box::new(|info| {
        restore_terminal();
        eprintln!("whiskd: {info}");
    }));

    let args: Vec<String> = std::env::args().skip(1).collect();
    let base = PathBuf::from(format!("/tmp/whiskd-{}", unsafe { libc::getuid() }));
    let sub = args.first().map(String::as_str);
    match sub {
        Some("--version" | "-v") => return println!("whiskd {VERSION}"),
        None | Some("--help" | "-h") => return help(&base),
        _ => {}
    }
    let cwd = std::env::current_dir().unwrap_or_else(|e| die(&format!("cannot read cwd: {e}")));
    let dir = base.join(format!("{:016x}", fnv1a(cwd.as_os_str().as_bytes())));
    let _ = CTX.set(Ctx { base, dir, cwd });
    housekeep();

    let rest = &args[1..];
    let flag = |f: &[&str]| rest.iter().any(|a| f.contains(&a.as_str()));
    match sub.unwrap_or_default() {
        "status" => cmd_status(flag(&["--json"])),
        "stop" => cmd_stop(rest.first().map(String::as_str)),
        "clean" => cmd_clean(),
        "logs" => cmd_logs(rest),
        "top" => cmd_top(flag(&["--global", "-g"])),
        "attach" => cmd_attach(rest.first().map(String::as_str)),
        "start" => cmd_start(rest),
        _ => cmd_fg(&args),
    }
    exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn pure_helpers() {
        let _ = CTX.set(Ctx { base: "/tmp/x".into(), dir: "/tmp/x/y".into(), cwd: "/work/myapp".into() });

        assert_eq!(shell_quote(&v(&["npm run dev && x"])), "npm run dev && x");
        assert_eq!(shell_quote(&v(&["printf", "%s\n", "a b"])), "printf '%s\n' 'a b'");
        assert_eq!(shell_quote(&v(&["FOO=a b", "cmd"])), "FOO='a b' cmd");
        assert_eq!(shell_quote(&v(&["it's", ""])), "'it'\\''s' ''");

        for (cmd, want) in [
            ("npm run dev", "dev"),
            ("yarn run build", "build"),
            ("bun run dev", "dev"),
            ("node --watch server.js", "server"),
            ("python3 app.py", "app"),
            ("deno run -A main.ts", "main"),
            ("FOO=1 tail -f x.log", "tail"),
            ("go run .", "myapp"),
        ] {
            assert_eq!(derive_name(&v(&[cmd])), want, "{cmd}");
        }
        assert_eq!(derive_name(&v(&["npm", "run", "dev"])), "dev");

        assert_eq!(parse_name_flag(&v(&["tail", "-n", "3"])).0, None);
        assert_eq!(parse_name_flag(&v(&["-n", "API", "x"])), (Some("api".into()), v(&["x"])));
        assert_eq!(sanitize("../My API"), "---my-api");

        assert_eq!(parse_etime("05:30"), Some(330));
        assert_eq!(parse_etime("1-02:03:04"), Some(93784));
        assert_eq!(parse_etime("junk"), None);

        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_700_000_000_123), "2023-11-14T22:13:20.123Z");

        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m \x1b]0;t\x07ok"), "red ok");
        assert_eq!(clean_meta("a\x1b[2J\n\nb "), "a b");
        assert_eq!(js("a\"b\n"), "\"a\\\"b\\u000a\"");
    }
}
