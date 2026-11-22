//! Profiler scopes on [puffin](https://crates.io/crates/puffin).
//!
//! With the `profile` feature off every item here is a no-op that compiles to
//! nothing: [`profile_scope!`] expands to nothing, [`SystemScope`] and
//! [`Guard`] are zero-sized. With it on, the scopes are recorded only while
//! [`set_enabled`] is `true` (the editor's Profiler panel turns it on; games
//! can set `OLVIN_PROFILE=1` or call it), so an idle profiler costs one atomic
//! load per scope.
//!
//! Frames are cut by [`new_frame`], which [`crate::App::update`] calls itself
//! unless the host owns frame boundaries (the editor does).

#[cfg(not(feature = "profile"))]
pub use off::*;
#[cfg(feature = "profile")]
pub use on::*;

/// Times the rest of the enclosing block as a scope named `$name` (a string
/// literal, shown in the profiler). Expands to nothing without the `profile`
/// feature of `olvin_core`.
#[cfg(feature = "profile")]
#[macro_export]
macro_rules! profile_scope {
    ($name:literal) => {
        let _olvin_profile_scope = $crate::profile::puffin::profile_scope_custom!($name);
    };
}

#[cfg(not(feature = "profile"))]
#[macro_export]
macro_rules! profile_scope {
    ($name:literal) => {};
}

/// Name of a system for the profiler: its type path without the closure suffix.
pub fn system_name<T>() -> &'static str {
    let name = std::any::type_name::<T>();
    name.strip_suffix("::{{closure}}").unwrap_or(name)
}

#[cfg(not(feature = "profile"))]
mod off {
    /// Scope id of one system. Zero-sized without the `profile` feature.
    #[derive(Clone, Copy, Default)]
    pub struct SystemScope;

    /// Ends the scope when dropped.
    pub struct Guard;

    impl SystemScope {
        #[inline(always)]
        pub fn register(_name: &'static str) -> Self {
            Self
        }

        #[inline(always)]
        pub fn enter(&self) -> Guard {
            Guard
        }
    }

    #[inline(always)]
    pub fn set_enabled(_on: bool) {}

    #[inline(always)]
    pub fn is_enabled() -> bool {
        false
    }

    #[inline(always)]
    pub fn new_frame() {}

    #[inline(always)]
    pub fn init_from_env() {}
}

#[cfg(feature = "profile")]
mod on {
    use std::sync::OnceLock;

    pub use puffin;

    /// Scope id of one system, registered once when the system is added.
    #[derive(Clone, Copy)]
    pub struct SystemScope(puffin::ScopeId);

    /// Ends the scope when dropped.
    pub struct Guard(#[allow(dead_code)] Option<puffin::ProfilerScope>);

    impl SystemScope {
        pub fn register(name: &'static str) -> Self {
            Self(puffin::ThreadProfiler::call(|tp| {
                tp.register_named_scope(name, "", "", 0)
            }))
        }

        #[inline]
        pub fn enter(&self) -> Guard {
            Guard(puffin::are_scopes_on().then(|| puffin::ProfilerScope::new(self.0, "")))
        }
    }

    /// Records scopes (or stops).
    pub fn set_enabled(on: bool) {
        puffin::set_scopes_on(on);
    }

    pub fn is_enabled() -> bool {
        puffin::are_scopes_on()
    }

    /// Ends the current profiler frame and starts the next one.
    pub fn new_frame() {
        if puffin::are_scopes_on() {
            puffin::GlobalProfiler::lock().new_frame();
        }
    }

    /// `OLVIN_PROFILE=1` turns recording on; with the `profile-http` feature
    /// `OLVIN_PROFILE_HTTP=1` (or an `addr:port`) also serves it to
    /// `puffin_viewer`. Called once by [`crate::App::new`].
    pub fn init_from_env() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            if std::env::var_os("OLVIN_PROFILE").is_some_and(|v| !v.is_empty() && v != "0") {
                set_enabled(true);
            }
            #[cfg(feature = "profile-http")]
            if let Some(v) = std::env::var_os("OLVIN_PROFILE_HTTP").filter(|v| !v.is_empty()) {
                let addr = match v.to_string_lossy().as_ref() {
                    "1" => format!("127.0.0.1:{}", puffin_http::DEFAULT_PORT),
                    other => other.to_owned(),
                };
                serve(&addr);
            }
        });
    }

    /// Serves the profile on `addr` for `puffin_viewer` (cargo install
    /// puffin_viewer), and turns recording on. One server per process.
    #[cfg(feature = "profile-http")]
    pub fn serve(addr: &str) {
        static SERVER: std::sync::Mutex<Option<puffin_http::Server>> = std::sync::Mutex::new(None);
        let mut server = SERVER.lock().unwrap_or_else(|e| e.into_inner());
        if server.is_some() {
            return;
        }
        match puffin_http::Server::new(addr) {
            Ok(s) => {
                log::info!("profiler: serving puffin on {addr}");
                *server = Some(s);
                set_enabled(true);
            }
            Err(e) => log::warn!("profiler: cannot serve on {addr}: {e}"),
        }
    }

    /// One scope of a captured frame.
    #[derive(Clone, Debug, PartialEq)]
    pub struct ScopeTiming {
        pub name: String,
        /// 0 for a thread's outermost scopes.
        pub depth: u32,
        /// Index into [`FrameProfile::threads`].
        pub thread: usize,
        /// From the start of the frame.
        pub start_ns: i64,
        pub duration_ns: i64,
    }

    /// One captured frame, flattened.
    #[derive(Clone, Debug, Default)]
    pub struct FrameProfile {
        pub index: u64,
        pub duration_ns: i64,
        pub threads: Vec<String>,
        /// Parents before their children, per thread in start order.
        pub scopes: Vec<ScopeTiming>,
    }

    /// Receives every frame [`new_frame`] closes; keeps the recent ones.
    pub struct Collector {
        view: puffin::GlobalFrameView,
    }

    impl Default for Collector {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Collector {
        pub fn new() -> Self {
            Self {
                view: puffin::GlobalFrameView::default(),
            }
        }

        /// The newest closed frame.
        pub fn latest(&self) -> Option<FrameProfile> {
            let view = self.view.lock();
            let frame = view.latest_frame()?;
            flatten(&frame, view.scope_collection())
        }

        /// Up to `n` newest closed frames, oldest first.
        pub fn recent(&self, n: usize) -> Vec<FrameProfile> {
            let view = self.view.lock();
            view.latest_frames(n)
                .filter_map(|f| flatten(f, view.scope_collection()))
                .collect()
        }
    }

    fn flatten(
        frame: &puffin::FrameData,
        scopes: &puffin::ScopeCollection,
    ) -> Option<FrameProfile> {
        let unpacked = frame.unpacked().ok()?;
        let base = frame.range_ns().0;
        let mut out = FrameProfile {
            index: frame.frame_index(),
            duration_ns: frame.duration_ns(),
            ..Default::default()
        };
        for (info, stream_info) in &unpacked.thread_streams {
            out.threads.push(info.name.clone());
            let thread = out.threads.len() - 1;
            walk(
                &stream_info.stream,
                0,
                0,
                &Ctx {
                    scopes,
                    base,
                    thread,
                },
                &mut out.scopes,
            );
        }
        Some(out)
    }

    struct Ctx<'a> {
        scopes: &'a puffin::ScopeCollection,
        base: i64,
        thread: usize,
    }

    fn walk(
        stream: &puffin::Stream,
        offset: u64,
        depth: u32,
        ctx: &Ctx,
        out: &mut Vec<ScopeTiming>,
    ) {
        let Ok(reader) = puffin::Reader::with_offset(stream, offset) else {
            return;
        };
        for scope in reader.flatten() {
            let name = ctx
                .scopes
                .fetch_by_id(&scope.id)
                .map_or_else(|| "?".to_owned(), |d| d.name().to_string());
            out.push(ScopeTiming {
                name,
                depth,
                thread: ctx.thread,
                start_ns: scope.record.start_ns - ctx.base,
                duration_ns: scope.record.duration_ns,
            });
            walk(stream, scope.child_begin_position, depth + 1, ctx, out);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn collects_named_scopes_with_nesting() {
            let collector = Collector::new();
            set_enabled(true);
            new_frame();
            {
                crate::profile_scope!("test.outer");
                let sys = SystemScope::register("test.system");
                let _g = sys.enter();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            new_frame();
            // Other tests may close frames too: look through the recent ones.
            let hit = collector.recent(64).into_iter().any(|f| {
                let sys = f.scopes.iter().find(|s| s.name == "test.system");
                let outer = f.scopes.iter().find(|s| s.name == "test.outer");
                matches!((sys, outer), (Some(s), Some(o))
                    if s.depth == o.depth + 1 && s.duration_ns >= 1_000_000)
            });
            assert!(hit, "scopes recorded with nesting and timing");
        }

        fn probe_system(_: &mut crate::Resources) {}

        #[test]
        fn schedule_names_systems_and_stages() {
            let collector = Collector::new();
            set_enabled(true);
            let mut app = crate::App::new();
            app.add_system(crate::Stage::Update, probe_system);
            app.update();
            app.update();
            let hit = collector.recent(64).into_iter().any(|f| {
                f.scopes.iter().any(|s| s.name.ends_with("::probe_system"))
                    && f.scopes.iter().any(|s| s.name == "stage Update")
            });
            assert!(hit, "system and stage scopes recorded");
        }
    }
}
