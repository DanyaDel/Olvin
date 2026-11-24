//! Единый логгер движка и перехват паник. Без внешних зависимостей, кроме `log`.
//!
//! Уровни задаются фильтром как у `env_logger`: `"info,wgpu_core=warn,olvin_render=debug"`.
//! Переменная окружения `OLVIN_LOG` (или `RUST_LOG`) переопределяет фильтр по умолчанию.
//! Строки идут в stderr и, если задан файл, туда же. При панике рядом с логом
//! пишется отчёт `crash-<unix-время>.txt`: сообщение, место, поток, backtrace
//! и последние строки лога. Относительные пути считаются от папки проекта
//! ([`ProjectPaths::detect`](crate::ProjectPaths::detect)), а не от рабочей папки.
//!
//! Приватность (S3). Строки лога, отчёты о панике и сообщения о них проходят через
//! [`Redactor`]: домашняя папка пишется как `~`, папка проекта как `<project>`,
//! имя пользователя ОС как `<user>`. Так лог и `crash-*.txt` можно приложить к
//! багу, не раскрыв имя и раскладку папок. Это чистка текста на нашей стороне, а
//! не защита: пути внутри бинарника убирает `olvin-hub build` (ремап путей), а
//! вывод стандартного хука паники в stderr (его зовёт наш хук следом) мы не
//! правим. Ничего никуда не отправляется: логи и отчёты — только локальные
//! файлы, в движке нет ни сетевого клиента, ни телеметрии.
//!
//! ```no_run
//! olvin_core::logging::Logging::new("info,wgpu_core=warn")
//!     .file("logs/olvin.log")
//!     .init();
//! ```

use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use log::{LevelFilter, Log, Metadata, Record};

/// Сколько последних строк лога попадает в отчёт о панике.
const TAIL_LINES: usize = 64;

/// Настройка логгера. `init()` ставит его глобально (один раз на процесс).
#[derive(Clone, Debug)]
pub struct Logging {
    filter: String,
    file: Option<PathBuf>,
    crash_dir: Option<PathBuf>,
    panic_hook: bool,
    redactor: Option<Redactor>,
    redact: bool,
}

impl Logging {
    /// `default_filter` используется, если не заданы `OLVIN_LOG` и `RUST_LOG`.
    pub fn new(default_filter: &str) -> Self {
        let filter = std::env::var("OLVIN_LOG")
            .or_else(|_| std::env::var("RUST_LOG"))
            .unwrap_or_else(|_| default_filter.to_owned());
        Self {
            filter,
            file: None,
            crash_dir: None,
            panic_hook: true,
            redactor: None,
            redact: true,
        }
    }

    /// Не чистить пути и имя пользователя (для отладки у себя; по умолчанию чистятся).
    pub fn without_redaction(mut self) -> Self {
        self.redact = false;
        self
    }

    /// Свои правила чистки вместо взятых из окружения (тесты).
    pub fn redactor(mut self, redactor: Redactor) -> Self {
        self.redactor = Some(redactor);
        self
    }

    /// Дублировать лог в файл (перезаписывается при запуске, папки создаются).
    /// Отчёты о паниках по умолчанию кладутся в ту же папку. Относительный
    /// путь — от папки проекта (`ProjectPaths`).
    pub fn file(mut self, path: impl Into<PathBuf>) -> Self {
        self.file = Some(path.into());
        self
    }

    /// Папка для отчётов о паниках. Без неё и без `file` — папка проекта.
    pub fn crash_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.crash_dir = Some(dir.into());
        self
    }

    /// Не ставить перехват паник (например, в тестах).
    pub fn without_panic_hook(mut self) -> Self {
        self.panic_hook = false;
        self
    }

    /// Ставит логгер и перехват паник. `false`, если логгер уже был установлен
    /// (тогда ничего не меняется).
    pub fn init(mut self) -> bool {
        let filter = Filter::parse(&self.filter);
        let paths = crate::ProjectPaths::detect();
        self.file = self.file.map(|f| paths.resolve(f));
        self.crash_dir = self.crash_dir.map(|d| paths.resolve(d));
        let file = self.file.as_deref().and_then(|path| {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                let _ = fs::create_dir_all(dir);
            }
            match File::create(path) {
                Ok(f) => Some(f),
                Err(err) => {
                    eprintln!("olvin: не удалось открыть лог {}: {err}", path.display());
                    None
                }
            }
        });
        let crash_dir = self
            .crash_dir
            .clone()
            .or_else(|| {
                self.file
                    .as_deref()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
            })
            .unwrap_or_else(|| paths.root.clone());

        let redactor = match (self.redact, self.redactor.take()) {
            (false, _) => Redactor::default(),
            (true, Some(custom)) => custom,
            (true, None) => Redactor::from_env(),
        };
        let max = filter.max_level();
        let hook_redactor = redactor.clone();
        let logger = LOGGER.get_or_init(|| Logger {
            redactor,
            filter,
            start: Instant::now(),
            out: Mutex::new(Output {
                file,
                tail: VecDeque::with_capacity(TAIL_LINES),
            }),
        });
        if log::set_logger(logger).is_err() {
            return false;
        }
        log::set_max_level(max);
        if self.panic_hook {
            install_panic_hook(crash_dir, hook_redactor);
        }
        true
    }
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

struct Logger {
    redactor: Redactor,
    filter: Filter,
    start: Instant,
    out: Mutex<Output>,
}

struct Output {
    file: Option<File>,
    tail: VecDeque<String>,
}

impl Log for Logger {
    fn enabled(&self, meta: &Metadata) -> bool {
        meta.level() <= self.filter.level_for(meta.target())
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "[{:>9.3} {:<5} {}] {}",
            self.start.elapsed().as_secs_f64(),
            record.level(),
            record.target(),
            record.args()
        );
        let line = self.redactor.redact(&line).into_owned();
        eprintln!("{line}");
        let Ok(mut out) = self.out.lock() else { return };
        if let Some(file) = out.file.as_mut() {
            let _ = writeln!(file, "{line}");
        }
        if out.tail.len() == TAIL_LINES {
            out.tail.pop_front();
        }
        out.tail.push_back(line);
    }

    fn flush(&self) {
        if let Ok(mut out) = self.out.lock()
            && let Some(file) = out.file.as_mut()
        {
            let _ = file.flush();
        }
    }
}

/// Фильтр уровней: общий уровень и уровни по префиксу цели (модуля).
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    default: LevelFilter,
    /// Отсортированы от длинного префикса к короткому.
    targets: Vec<(String, LevelFilter)>,
}

impl Filter {
    /// `"warn,olvin_render=debug,wgpu_core=off"`. Неизвестные части пропускаются.
    pub fn parse(spec: &str) -> Self {
        let mut default = LevelFilter::Error;
        let mut targets = Vec::new();
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some((target, level)) => {
                    if let Ok(level) = level.trim().parse() {
                        targets.push((target.trim().to_owned(), level));
                    }
                }
                None => match part.parse() {
                    Ok(level) => default = level,
                    // Голое имя модуля, как в env_logger, включает всё.
                    Err(_) => targets.push((part.to_owned(), LevelFilter::Trace)),
                },
            }
        }
        targets.sort_by_key(|(t, _)| std::cmp::Reverse(t.len()));
        Self { default, targets }
    }

    /// Уровень для цели `target` (`olvin_render::core` подходит под `olvin_render`).
    pub fn level_for(&self, target: &str) -> LevelFilter {
        self.targets
            .iter()
            .find(|(prefix, _)| {
                target
                    .strip_prefix(prefix.as_str())
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
            })
            .map_or(self.default, |(_, level)| *level)
    }

    /// Самый подробный уровень из всех: дешёвая отсечка в макросах `log`.
    pub fn max_level(&self) -> LevelFilter {
        self.targets
            .iter()
            .map(|(_, l)| *l)
            .fold(self.default, Ord::max)
    }
}

/// Чистит из текста то, что выдаёт человека и его машину: папку проекта
/// (`<project>`), домашнюю папку (`~`) и имя пользователя ОС (`<user>`).
///
/// * Пути заменяются целиком по границе компонента: `/home/ann` не заденет
///   `/home/annabel`. Подходят обе записи разделителей (`/` и `\`), а также
///   канонический путь, если он отличается (симлинки). Раньше идёт более
///   длинный путь, поэтому проект внутри домашней папки станет `<project>/src/…`.
/// * Имя пользователя заменяется как целое слово (не внутри `abundant`), только
///   если оно не короче трёх символов и не `root`: иначе чистка портила бы текст.
/// * Корень (`/`, `C:\`) и пустые значения не используются как образец.
///
/// Это чистка текста лога, а не защита: см. документацию модуля.
#[derive(Clone, Debug, Default)]
pub struct Redactor {
    /// Образец и замена, от длинного образца к короткому.
    paths: Vec<(String, &'static str)>,
    user: Option<String>,
}

impl Redactor {
    /// Правила из заданных значений (тесты, свои хосты). Любое можно не задавать.
    pub fn new(home: Option<&str>, user: Option<&str>, project: Option<&str>) -> Self {
        let mut paths: Vec<(String, &'static str)> = Vec::new();
        let mut add = |path: &str, to: &'static str| {
            let path = path.trim_end_matches(['/', '\\']);
            // Корень после обрезки разделителей пуст; одна буква диска тоже не образец.
            if path.len() < 2 || path.ends_with(':') {
                return;
            }
            let mut spellings = vec![path.to_owned(), path.replace('\\', "/")];
            spellings.push(path.replace('/', "\\"));
            if let Ok(real) = Path::new(path).canonicalize()
                && let Some(real) = real.to_str()
            {
                spellings.push(real.to_owned());
                spellings.push(real.replace('/', "\\"));
            }
            for spelling in spellings {
                if !paths.iter().any(|(p, _)| *p == spelling) {
                    paths.push((spelling, to));
                }
            }
        };
        // Проект раньше дома: он обычно внутри и должен выиграть.
        if let Some(project) = project {
            add(project, "<project>");
        }
        if let Some(home) = home {
            add(home, "~");
        }
        paths.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));

        // Имя берём из `user`, а если его нет, из `/home/<имя>` или `C:\Users\<имя>`.
        let from_home = home.and_then(|h| {
            let h = h.trim_end_matches(['/', '\\']);
            let (parent, name) = h.rsplit_once(['/', '\\'])?;
            let parent = parent.rsplit(['/', '\\']).next().unwrap_or_default();
            matches!(parent, "home" | "Users").then_some(name)
        });
        let user = user
            .filter(|u| !u.is_empty())
            .or(from_home)
            .filter(|u| u.chars().count() >= 3 && *u != "root")
            .map(str::to_owned);
        Self { paths, user }
    }

    /// Домашняя папка и имя из окружения (`HOME`/`USERPROFILE`, `USER`/`USERNAME`/`LOGNAME`),
    /// папка проекта из [`ProjectPaths::detect`](crate::ProjectPaths::detect).
    pub fn from_env() -> Self {
        let var = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
        };
        let home = var(&["HOME", "USERPROFILE"]);
        let user = var(&["USER", "USERNAME", "LOGNAME"]);
        let project = crate::ProjectPaths::detect()
            .root
            .to_string_lossy()
            .into_owned();
        Self::new(home.as_deref(), user.as_deref(), Some(&project))
    }

    /// Текст без путей и имени. Без совпадений ничего не копируется.
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut out = Cow::Borrowed(text);
        for (path, to) in &self.paths {
            if out.contains(path.as_str()) {
                out = Cow::Owned(replace_bounded(&out, path, to, path_end_ok));
            }
        }
        if let Some(user) = &self.user
            && out.contains(user.as_str())
        {
            out = Cow::Owned(replace_bounded(&out, user, "<user>", word_end_ok));
        }
        out
    }
}

/// После пути должен идти разделитель или не-имя: `/home/ann` не часть `/home/annabel`.
fn path_end_ok(next: Option<char>) -> bool {
    next.is_none_or(|c| {
        matches!(c, '/' | '\\') || !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
    })
}

fn word_end_ok(next: Option<char>) -> bool {
    next.is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
}

/// Заменяет вхождения `needle` на `to`, если перед ними не буква или цифра (`before_ok`
/// для пути ослаблено ниже) и после них `after_ok`.
fn replace_bounded(
    text: &str,
    needle: &str,
    to: &str,
    after_ok: fn(Option<char>) -> bool,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, _) in text.match_indices(needle) {
        let end = start + needle.len();
        // Мы могли уже пройти это место перекрывающимся вхождением.
        if start < last {
            continue;
        }
        let before = text[..start].chars().next_back();
        let before_ok = before.is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
            || needle.starts_with(['/', '\\']);
        if before_ok && after_ok(text[end..].chars().next()) {
            out.push_str(&text[last..start]);
            out.push_str(to);
            last = end;
        }
    }
    out.push_str(&text[last..]);
    out
}

fn install_panic_hook(dir: PathBuf, redactor: Redactor) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let report = redactor.redact(&panic_report(info)).into_owned();
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = dir.join(format!("crash-{secs}.txt"));
        let _ = fs::create_dir_all(&dir);
        match fs::write(&path, &report) {
            Ok(()) => eprintln!(
                "olvin: отчёт о панике сохранён в {}",
                redactor.redact(&path.display().to_string())
            ),
            Err(err) => eprintln!(
                "olvin: не удалось сохранить отчёт о панике: {}",
                redactor.redact(&err.to_string())
            ),
        }
        log::logger().flush();
        previous(info);
    }));
}

fn panic_report(info: &std::panic::PanicHookInfo) -> String {
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<не строка>".into());
    let location = info
        .location()
        .map_or_else(|| "?".into(), |l| format!("{}:{}", l.file(), l.line()));
    let thread = std::thread::current();
    let mut out = String::new();
    let _ = writeln!(out, "Olvin: паника");
    let _ = writeln!(out, "сообщение: {message}");
    let _ = writeln!(out, "место: {location}");
    let _ = writeln!(out, "поток: {}", thread.name().unwrap_or("<без имени>"));
    let _ = writeln!(
        out,
        "система: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(
        out,
        "\nbacktrace:\n{}",
        std::backtrace::Backtrace::force_capture()
    );
    // try_lock: паника могла случиться, пока лог держал мьютекс.
    if let Some(logger) = LOGGER.get()
        && let Ok(o) = logger.out.try_lock()
    {
        let _ = writeln!(out, "последние строки лога:");
        for line in &o.tail {
            let _ = writeln!(out, "{line}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Level;

    #[test]
    fn filter_picks_longest_prefix_on_module_boundary() {
        let f = Filter::parse("info, wgpu_core=warn, wgpu_core::device=off ,olvin_render=debug");
        assert_eq!(f.level_for("sandbox"), LevelFilter::Info);
        assert_eq!(f.level_for("wgpu_core::present"), LevelFilter::Warn);
        assert_eq!(f.level_for("wgpu_core::device::life"), LevelFilter::Off);
        assert_eq!(f.level_for("olvin_render"), LevelFilter::Debug);
        // Префикс без границы модуля не считается.
        assert_eq!(f.level_for("olvin_render_extra"), LevelFilter::Info);
        assert_eq!(f.max_level(), LevelFilter::Debug);
    }

    #[test]
    fn filter_defaults_and_bare_targets() {
        assert_eq!(Filter::parse("").level_for("x"), LevelFilter::Error);
        let f = Filter::parse("warn,olvin_physics,bad=nonsense");
        assert_eq!(f.level_for("olvin_physics::query"), LevelFilter::Trace);
        assert_eq!(f.level_for("bad"), LevelFilter::Warn);
    }

    #[test]
    fn redactor_replaces_whole_paths_and_words_only() {
        let r = Redactor::new(Some("/home/ann"), Some("ann"), Some("/home/ann/dev/game"));
        assert_eq!(
            r.redact("open /home/ann/dev/game/assets/a.ron failed"),
            "open <project>/assets/a.ron failed"
        );
        assert_eq!(
            r.redact("/home/ann/.cargo/registry/x.rs:3"),
            "~/.cargo/registry/x.rs:3"
        );
        assert_eq!(r.redact("at /home/ann"), "at ~");
        assert_eq!(r.redact("(/home/ann/dev/game)"), "(<project>)");
        // Другой человек с похожим именем и слова с именем внутри остаются.
        assert_eq!(
            r.redact("/home/annabel/x ann-pc annual"),
            "/home/annabel/x <user>-pc annual"
        );
        assert_eq!(r.redact("/home/ann/dev/gamekit/x"), "~/dev/gamekit/x");
        // Имя в тексте и в `/home/<имя>` без заданной домашней папки.
        let only_user = Redactor::new(None, Some("ann"), None);
        assert_eq!(
            only_user.redact("/home/ann/x by ann"),
            "/home/<user>/x by <user>"
        );
        // Нечего чистить: строка не копируется.
        assert!(matches!(r.redact("nothing here"), Cow::Borrowed(_)));
    }

    #[test]
    fn redactor_handles_windows_paths_and_unsafe_values() {
        let r = Redactor::new(Some(r"C:\Users\Bob"), None, Some(r"C:\Users\Bob\proj"));
        assert_eq!(
            r.redact(r"C:\Users\Bob\proj\src\main.rs"),
            r"<project>\src\main.rs"
        );
        assert_eq!(r.redact("C:/Users/Bob/AppData/x"), "~/AppData/x");
        assert_eq!(
            r.redact("hello Bob"),
            "hello <user>",
            "name taken from C:\\Users\\<name>"
        );
        // Корень, короткие имена и root не используются: иначе портили бы весь текст.
        let r = Redactor::new(Some("/"), Some("al"), Some(""));
        assert_eq!(r.redact("/usr/bin al /"), "/usr/bin al /");
        let r = Redactor::new(Some("/root"), Some("root"), None);
        assert_eq!(r.redact("/root/x as root"), "~/x as root");
        assert!(Redactor::default().redact("/home/x").starts_with("/home"));
    }

    #[test]
    fn writes_file_and_panic_report() {
        let dir = std::env::temp_dir().join(format!("olvin_logging_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let log_path = dir.join("olvin.log");
        // Фальшивые HOME, имя и проект: настоящие значения машины не нужны.
        let fake = Redactor::new(
            Some("/home/zoe_test"),
            Some("zoe_test"),
            Some("/home/zoe_test/dev/game"),
        );
        assert!(
            Logging::new("info")
                .file(&log_path)
                .redactor(fake.clone())
                .without_panic_hook()
                .init()
        );
        log::info!(target: "olvin_test", "привет из теста");
        log::info!(target: "olvin_test", "нет файла /home/zoe_test/dev/game/assets/a.ron для zoe_test");
        log::debug!(target: "olvin_test", "скрыто фильтром");
        log::logger().flush();
        let text = fs::read_to_string(&log_path).unwrap();
        assert!(text.contains("INFO  olvin_test] привет из теста"), "{text}");
        assert!(!text.contains("скрыто"));
        assert!(
            text.contains("нет файла <project>/assets/a.ron для <user>"),
            "{text}"
        );
        assert!(!text.contains("zoe_test"), "{text}");
        assert!(log::log_enabled!(target: "olvin_test", Level::Warn));
        assert!(!log::log_enabled!(target: "olvin_test", Level::Debug));

        // Повторная установка ничего не ломает.
        assert!(!Logging::new("trace").without_panic_hook().init());

        // Отчёт о панике: ставим хук в отдельную папку, паникуем в потоке.
        let crash = dir.join("crashes");
        install_panic_hook(crash.clone(), fake);
        let joined = std::thread::Builder::new()
            .name("crasher".into())
            .spawn(|| panic!("проверка отчёта в /home/zoe_test/.cache от zoe_test"))
            .unwrap()
            .join();
        let _ = std::panic::take_hook();
        assert!(joined.is_err());
        let report = fs::read_dir(&crash)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .find(|r| r.contains("проверка отчёта"))
            .expect("отчёт о панике");
        assert!(
            report.contains("проверка отчёта в ~/.cache от <user>"),
            "{report}"
        );
        assert!(!report.contains("zoe_test"), "{report}");
        assert!(report.contains("поток: crasher"), "{report}");
        assert!(report.contains("привет из теста"));
        let _ = fs::remove_dir_all(&dir);
    }
}
