//! Home download recovery exercises the rendered action and the real installer.

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    time::{Duration, Instant},
};

use block_launcher::{
    ProxyMode,
    catalog::{Catalog, InstallStatus},
};

#[gpui::test]
fn home_retries_a_failed_download_without_an_account_or_a_second_instance(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let proxy = TcpListener::bind("127.0.0.1:0")?;
        proxy.set_nonblocking(true)?;
        let proxy_url = format!("http://{}", proxy.local_addr()?);
        let server = std::thread::spawn(move || -> anyhow::Result<String> {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match proxy.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                        let mut bytes = [0; 4096];
                        let length = stream.read(&mut bytes)?;
                        stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                        return Ok(String::from_utf8_lossy(&bytes[..length]).into_owned());
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        });
        let root = tempfile::tempdir()?;
        let mut preferences = crate::appearance::Preferences::default();
        preferences.downloads.proxy = ProxyMode::Http { url: proxy_url };
        preferences.save(root.path())?;
        let catalog = Catalog::open(root.path())?;
        let (instance, profile) =
            catalog.create_instance("Download failed".into(), "1.20.6".into(), None, None)?;
        let mut target = instance.targets[0].clone();
        target.status = InstallStatus::Failed {
            message: "Initial download failure".into(),
        };
        catalog.set_target(&instance.id, &target)?;
        let snapshot = catalog.snapshot()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = cx.add_window_view(crate::tests::test_view);
        cx.simulate_resize(gpui::size(gpui::px(432.), gpui::px(768.)));
        view.update(cx, |view, cx| {
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.snapshot = snapshot;
            view.selected_instance = Some(instance.id.clone());
            view.selected_profile = Some(profile.id.clone());
            view.tab = crate::Tab::Home;
            cx.notify();
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("home-play")
            .ok_or_else(|| anyhow::anyhow!("Home primary action is missing"))?;
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        assert!(view.read_with(cx, |view, _| view.busy && view.accounts.is_empty()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while view.read_with(cx, |view, _| view.busy) && Instant::now() < deadline {
            view.update(cx, crate::Block::poll);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!view.read_with(cx, |view, _| view.busy));
        assert!(view.read_with(cx, |view, _| view.tab == crate::Tab::Home
            && view.form.is_none()));
        let request = server
            .join()
            .map_err(|_| anyhow::anyhow!("loopback proxy worker failed"))??;
        assert!(request.starts_with("CONNECT piston-meta.mojang.com:443 "));
        let state = catalog.snapshot()?;
        assert_eq!(state.instances.len(), 1);
        assert_eq!(state.profiles.len(), 1);
        let (saved_instance, saved_profile, saved_target) = state.selection(&profile.id)?;
        assert_eq!(saved_instance.id, instance.id);
        assert_eq!(saved_instance.directory, instance.directory);
        assert_eq!(saved_profile.id, profile.id);
        assert!(
            matches!(&saved_target.status, InstallStatus::Failed { message } if message != "Initial download failure")
        );
        assert_eq!(cx.opened_url(), None);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}
