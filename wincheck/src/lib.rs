// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
//
// Scratch harness for PR #7464: runs the PR's icons.rs (included verbatim) on a
// Windows runner, plus diagnostics that print what each lookup finds.
pub mod icons {
    include!("../icons.rs");

    #[cfg(all(test, target_os = "windows"))]
    mod diag {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex;
        use std::time::{Duration, Instant};

        fn show(icon: &Result<Option<AppIcon>, String>) -> String {
            match icon {
                Ok(Some(i)) => format!("{} B png={} from {:?}", i.data.len(), i.data.starts_with(b"\x89PNG"), i.path),
                Ok(None) => "none".into(),
                Err(e) => format!("err: {e}"),
            }
        }

        #[tokio::test]
        async fn print_lookups() {
            let started = Instant::now();
            let packages = list_appx_packages().unwrap();
            println!("listed {} Store apps in {:?}", packages.len(), started.elapsed());
            assert!(!packages.is_empty());
            assert!(packages.iter().all(|(name, folder)| !name.is_empty() && folder.is_dir()), "{packages:?}");
            for name in [
                "ShellExperienceHost", "StartMenuExperienceHost", "SearchHost", "Calculator",
                "WindowsTerminal", "Notepad", "notepad.exe", "msedge", "msedge.exe", "explorer",
                "powershell", "Code", "Google Chrome", "chrome", "Firefox", "cmd", "x$(calc)",
                "Halo: Reach", ".exe", "a",
            ] {
                let started = Instant::now();
                let reg = get_exe_by_reg_key(name);
                let appx = get_exe_by_appx(name);
                let disk = get_exe_from_potential_path(name);
                let icon = get_app_icon(name, None).await;
                println!(
                    "{name:24} reg={reg:?}\n{:24} appx={appx:?}\n{:24} disk={disk:?}\n{:24} icon={} [{:?}]",
                    "", "", "", show(&icon), started.elapsed()
                );
            }
        }

        /// The package API returns the same apps, in the same order, as the
        /// Get-AppxPackage script the earlier revision of this PR ran (main
        /// packages only; frameworks hold no apps).
        #[test]
        fn matches_get_appx_package() {
            let script = "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); \
                Get-AppxPackage | ForEach-Object { $_.Name + \"`t\" + $_.InstallLocation + \"`t\" + $_.IsFramework }";
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .output()
                .unwrap();
            let all: Vec<(String, std::path::PathBuf, bool)> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| {
                    let mut parts = line.trim().split('\t');
                    let (name, folder, framework) = (parts.next()?, parts.next()?, parts.next()?);
                    Some((name.to_string(), folder.into(), framework == "True"))
                })
                .collect();
            let script_main: AppxPackages = all.iter().filter(|p| !p.2).map(|p| (p.0.clone(), p.1.clone())).collect();
            let api = list_appx_packages().unwrap();
            println!(
                "Get-AppxPackage: {} packages, {} main; PackageManager: {} main",
                all.len(), script_main.len(), api.len()
            );
            for (name, _) in script_main.iter().filter(|p| !api.contains(p)) {
                println!("  only in Get-AppxPackage: {name}");
            }
            for (name, _) in api.iter().filter(|p| !script_main.contains(p)) {
                println!("  only in PackageManager: {name}");
            }
            assert_eq!(api, script_main);
        }

        /// 100 lookups at once on a cold cache list once, with the real API.
        #[test]
        fn concurrent_lookups_list_once() {
            let slot = Mutex::new(None);
            let listings = AtomicUsize::new(0);
            let started = Instant::now();
            std::thread::scope(|scope| {
                for _ in 0..100 {
                    scope.spawn(|| {
                        let packages = cached_appx_packages(&slot, Duration::from_secs(300), || {
                            listings.fetch_add(1, Ordering::SeqCst);
                            list_appx_packages()
                        });
                        assert!(!packages.is_empty());
                    });
                }
            });
            println!("100 threads: {} listing(s) in {:?}", listings.load(Ordering::SeqCst), started.elapsed());
            assert_eq!(listings.load(Ordering::SeqCst), 1);
        }

        /// Run by the workflow as a new standard (non-admin) local user.
        #[tokio::test]
        #[ignore]
        async fn standard_user_lookup() {
            let groups = std::process::Command::new("whoami").arg("/groups").output().unwrap();
            let groups = String::from_utf8_lossy(&groups.stdout);
            println!(
                "user={} administrators_group={:?}",
                std::env::var("USERNAME").unwrap_or_default(),
                groups.lines().find(|l| l.contains("S-1-5-32-544")).map(str::trim)
            );
            let listing = list_appx_packages();
            println!("Store listing: {:?}", listing.as_ref().map(|p| p.len()));
            assert!(listing.is_ok());
            for name in ["notepad", "explorer", "msedge"] {
                println!("{name:10} {}", show(&get_app_icon(name, None).await));
            }
        }
    }
}

/// The PR's previous icons.rs (cae3cfe6d), to compare concurrent lookups.
#[cfg(target_os = "windows")]
pub mod icons_prev {
    include!("../icons_prev.rs");
}

#[cfg(all(test, target_os = "windows"))]
mod icon_concurrency {
    use std::time::Instant;

    const EXPLORER: &str = r"C:\Windows\explorer.exe";
    const NOTEPAD: &str = r"C:\Windows\notepad.exe";

    /// Calls `windows_icons::get_icon_by_path` from `threads` threads at once,
    /// `per_thread` times each, and returns how many calls failed.
    fn raw_failures(threads: usize, per_thread: usize, com: Option<windows::Win32::System::Com::COINIT>, lock: bool) -> (usize, Vec<String>) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let errors = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for t in 0..threads {
                let errors = &errors;
                scope.spawn(move || {
                    if let Some(model) = com {
                        unsafe { let _ = windows::Win32::System::Com::CoInitializeEx(None, model); }
                    }
                    for i in 0..per_thread {
                        let path = if (t + i) % 2 == 0 { EXPLORER } else { NOTEPAD };
                        let _guard = lock.then(|| LOCK.lock().unwrap());
                        if let Err(e) = windows_icons::get_icon_by_path(path) {
                            errors.lock().unwrap().push(format!("{path}: {e}"));
                        }
                    }
                });
            }
        });
        let errors = errors.into_inner().unwrap();
        (errors.len(), errors.into_iter().take(3).collect())
    }

    #[test]
    fn raw_get_icon_by_path_under_concurrency() {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED};
        for (label, threads, per, com, lock) in [
            ("1 thread, no COM", 1, 96, None, false),
            ("8 threads, no COM", 8, 12, None, false),
            ("8 threads, no COM, one at a time (mutex)", 8, 12, None, true),
            ("8 threads, COM STA per thread", 8, 12, Some(COINIT_APARTMENTTHREADED), false),
            ("8 threads, COM MTA per thread", 8, 12, Some(COINIT_MULTITHREADED), false),
            ("4 threads, no COM", 4, 24, None, false),
            ("2 threads, no COM", 2, 48, None, false),
        ] {
            let started = Instant::now();
            let (failed, sample) = raw_failures(threads, per, com, lock);
            println!("RAW {label:45} failed {failed}/{} in {:?} {sample:?}", threads * per, started.elapsed());
        }
    }

    async fn many_lookups<F, Fut>(label: &str, get: F)
    where
        F: Fn(&'static str) -> Fut,
        Fut: std::future::Future<Output = Result<Option<Vec<u8>>, String>> + Send + 'static,
    {
        let started = Instant::now();
        let handles: Vec<_> = (0..48)
            .map(|i| {
                let name = ["notepad", "explorer", "msedge"][i % 3];
                let lookup = get(name);
                tokio::spawn(async move { (name, lookup.await) })
            })
            .collect();
        let mut failed = Vec::new();
        for handle in handles {
            let (name, icon) = handle.await.unwrap();
            if !matches!(&icon, Ok(Some(data)) if data.starts_with(b"\x89PNG")) {
                failed.push(format!("{name}: {:?}", icon.map(|i| i.map(|d| d.len()))));
            }
        }
        println!("APP {label:45} failed {}/48 in {:?} {:?}", failed.len(), started.elapsed(), failed.iter().take(3).collect::<Vec<_>>());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn get_app_icon_under_concurrency_new_vs_prev() {
        for round in 0..3 {
            many_lookups(&format!("this commit, round {round}"), |name| async move {
                crate::icons::get_app_icon(name, None).await.map(|i| i.map(|i| i.data))
            })
            .await;
            many_lookups(&format!("previous commit cae3cfe6d, round {round}"), |name| async move {
                crate::icons_prev::get_app_icon(name, None).await.map(|i| i.map(|i| i.data))
            })
            .await;
        }
    }
}
