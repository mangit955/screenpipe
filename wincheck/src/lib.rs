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
            let packages = list_appx_packages().await;
            println!("listed {} Store packages in {:?}", packages.len(), started.elapsed());
            for (name, folder) in packages.iter().take(12) {
                println!("  {name}\t{}", folder.display());
            }
            for name in [
                "ShellExperienceHost", "StartMenuExperienceHost", "SearchHost", "Calculator",
                "WindowsTerminal", "Notepad", "notepad.exe", "msedge", "msedge.exe", "explorer",
                "powershell", "Code", "Google Chrome", "chrome", "Firefox", "cmd", "x$(calc)", ".exe", "a",
            ] {
                let started = Instant::now();
                let reg = get_exe_by_reg_key(name);
                let appx = get_exe_by_appx(name).await;
                let disk = get_exe_from_potential_path(name);
                let icon = get_app_icon(name, None).await;
                println!(
                    "{name:24} reg={reg:?}\n{:24} appx={appx:?}\n{:24} disk={disk:?}\n{:24} icon={} [{:?}]",
                    "", "", "", show(&icon), started.elapsed()
                );
            }
        }

        /// Real PowerShell behind the shared cache: 100 callers, a third of them
        /// giving up after 50 ms, start exactly one listing.
        #[tokio::test]
        async fn burst_with_aborts_lists_once() {
            static LISTINGS: AtomicUsize = AtomicUsize::new(0);
            let slot = std::sync::Mutex::new(None);
            let ttl = Duration::from_secs(300);
            let started = Instant::now();
            let callers = (0..100).map(|i| {
                let call = cached_appx_packages(&slot, ttl, ttl, || async {
                    LISTINGS.fetch_add(1, Ordering::SeqCst);
                    list_appx_packages().await
                });
                async move {
                    if i % 3 == 0 {
                        tokio::time::timeout(Duration::from_millis(50), call).await.ok().map(|p| p.len())
                    } else {
                        Some(call.await.len())
                    }
                }
            });
            let results = futures::future::join_all(callers).await;
            let finished: Vec<_> = results.iter().flatten().collect();
            println!(
                "100 callers ({} gave up): {} PowerShell listing(s), {} packages, {:?}",
                results.iter().filter(|r| r.is_none()).count(),
                LISTINGS.load(Ordering::SeqCst),
                finished.first().copied().copied().unwrap_or(0),
                started.elapsed()
            );
            assert_eq!(LISTINGS.load(Ordering::SeqCst), 1);
            assert!(finished.iter().all(|n| **n > 0 && *n == finished[0]));
        }

        /// The production listing failing for real (powershell.exe not found): the
        /// failure is reused for APPX_RETRY_AFTER, then a working listing replaces it.
        /// Run alone (`cargo test failed_listing`) so nothing has filled the cache.
        #[tokio::test]
        async fn failed_listing_is_retried() {
            let root = std::env::var_os("SystemRoot").unwrap();
            std::env::set_var("SystemRoot", r"C:\no-such-windows");
            let started = Instant::now();
            assert_eq!(get_exe_by_appx("ShellExperienceHost").await, None);
            std::env::set_var("SystemRoot", &root);
            println!("failed listing after {:?}", started.elapsed());
            // PowerShell works again, but the failure is still reused.
            assert_eq!(get_exe_by_appx("ShellExperienceHost").await, None);
            tokio::time::sleep(APPX_RETRY_AFTER.saturating_sub(started.elapsed()) + Duration::from_secs(1)).await;
            let found = get_exe_by_appx("ShellExperienceHost").await;
            println!("after {:?}: {found:?}", started.elapsed());
            assert!(found.is_some());
            let icon = get_app_icon("ShellExperienceHost", None).await;
            println!("icon: {}", show(&icon));
            assert!(matches!(icon, Ok(Some(ref i)) if i.data.starts_with(b"\x89PNG")));
        }

        /// What bytes PowerShell 5.1 writes for a non-ASCII path when launched the
        /// way list_appx_packages launches it.
        #[tokio::test]
        async fn powershell_output_encoding() {
            let text = "'C:\\Users\\J' + [char]0xFC + 'rgen\\' + [char]0x65E5 + '\\app'";
            let scripts = [
                ("default", text.to_string()),
                ("OutputEncoding utf8", format!("[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); {text}")),
                ("OutputEncoding UTF8 prop", format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8; {text}")),
                ("raw bytes", format!("$b = [Text.Encoding]::UTF8.GetBytes({text}); [Console]::OpenStandardOutput().Write($b, 0, $b.Length)")),
            ];
            for (label, script) in scripts {
                let mut command = tokio::process::Command::new(powershell_exe());
                command
                    .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "hidden"])
                    .args(["-Command", &script])
                    .creation_flags(0x08000000);
                let out = command.output().await.unwrap();
                println!(
                    "{label:26} status={:?} utf8={:?}\n{:26} hex={:02x?}\n{:26} stderr={:?}",
                    out.status.code(),
                    std::str::from_utf8(&out.stdout).ok(),
                    "",
                    out.stdout,
                    "",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }
}
