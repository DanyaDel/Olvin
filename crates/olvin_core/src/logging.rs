//! Единый логгер движка и перехват паник. Без внешних зависимостей, кроме `log`.
//!
//! Уровни задаются фильтром как у `env_logger`: `"info,wgpu_core=warn,olvin_render=debug"`.
//! Переменная окружения `OLVIN_LOG` (или `RUST_LOG`) переопределяет фильтр по умолчанию.
//! Строки идут в stderr и, если задан файл, туда же. При панике рядом с логом
//! пишется отчёт `crash-<unix-время>.txt`: сообщение, место, поток, backtrace
//! и последние строки лога.
//!
//! ```no_run
//! olvin_core::logging::Logging::new("info,wgpu_core=warn")
//!     .file("logs/olvin.log")
//!     .init();
//! ```

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
        }
    }

    /// Дублировать лог в файл (перезаписывается при запуске, папки создаются).
    /// Отчёты о паниках по умолчанию кладутся в ту же папку.
    pub fn file(mut self, path: impl Into<PathBuf>) -> Self {
        self.file = Some(path.into());
        self
    }

    /// Папка для отчётов о паниках. Без неё и без `file` — текущая папка.
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
    pub fn init(self) -> bool {
        let filter = Filter::parse(&self.filter);
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
            .unwrap_or_default();

        let max = filter.max_level();
        let logger = LOGGER.get_or_init(|| Logger {
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
            install_panic_hook(crash_dir);
        }
        true
    }
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

struct Logger {
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

fn install_panic_hook(dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let report = panic_report(info);
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = dir.join(format!("crash-{secs}.txt"));
        let _ = fs::create_dir_all(&dir);
        match fs::write(&path, &report) {
            Ok(()) => eprintln!("olvin: отчёт о панике сохранён в {}", path.display()),
            Err(err) => eprintln!("olvin: не удалось сохранить отчёт о панике: {err}"),
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
    fn writes_file_and_panic_report() {
        let dir = std::env::temp_dir().join(format!("olvin_logging_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let log_path = dir.join("olvin.log");
        assert!(
            Logging::new("info")
                .file(&log_path)
                .without_panic_hook()
                .init()
        );
        log::info!(target: "olvin_test", "привет из теста");
        log::debug!(target: "olvin_test", "скрыто фильтром");
        log::logger().flush();
        let text = fs::read_to_string(&log_path).unwrap();
        assert!(text.contains("INFO  olvin_test] привет из теста"), "{text}");
        assert!(!text.contains("скрыто"));
        assert!(log::log_enabled!(target: "olvin_test", Level::Warn));
        assert!(!log::log_enabled!(target: "olvin_test", Level::Debug));

        // Повторная установка ничего не ломает.
        assert!(!Logging::new("trace").without_panic_hook().init());

        // Отчёт о панике: ставим хук в отдельную папку, паникуем в потоке.
        let crash = dir.join("crashes");
        install_panic_hook(crash.clone());
        let joined = std::thread::Builder::new()
            .name("crasher".into())
            .spawn(|| panic!("проверка отчёта"))
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
        assert!(report.contains("поток: crasher"), "{report}");
        assert!(report.contains("привет из теста"));
        let _ = fs::remove_dir_all(&dir);
    }
}
