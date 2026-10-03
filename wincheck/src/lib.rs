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
