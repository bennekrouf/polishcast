//! What the window keeps doing in the background: checking the licence, keeping the shared
//! library in step with the takes, and coming forward when asked to.

use super::*;

pub(super) fn use_licence_check(pro: crate::licence::Pro) {
    // The licence is checked with api0 at start, then once a day.
    use_future(move || async move {
        let mut status = pro.status;
        status.set(desktop_kit::licence::refresh(true).await);
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            status.set(desktop_kit::licence::refresh(false).await);
        }
    });
}

/// A take opened from PublishKit or TrackCut is handed to `open_take`.
pub(super) fn use_library_sync(mut open_take: impl FnMut(String) + Copy + 'static) {
    // The takes in the shared library, kept as they are on disk: read again whenever a take or
    // an export comes, goes or changes.
    use_future(move || async move {
        let mut last = None;
        loop {
            let stamp = tokio::task::spawn_blocking(crate::library::stamp).await.ok();
            if stamp != last {
                let _ = tokio::task::spawn_blocking(crate::library::sync_all).await;
                last = stamp;
            }
            if let Some(note) =
                tokio::task::spawn_blocking(|| desktop_kit::handoff::take("polishcast", "open")).await.ok().flatten()
            {
                if let Some(target) = note["open"].as_str() {
                    open_take(target.to_string());
                }
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    });
}

pub(super) fn use_show_requests() {
    // Launched again while running (Start menu, a shortcut): the other launch handed over and
    // quit, and this window comes up instead.
    use_future(|| async {
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if crate::instance::take_show_request() {
                window().set_visible(true);
                window().set_minimized(false);
                window().set_focus();
            }
        }
    });
    // Clicking the app's icon (Dock, Finder, Launchpad) while it runs with the window closed:
    // macOS asks the app to reopen, and Dioxus leaves that unanswered.
    use_wry_event_handler(|event, _| {
        if let TaoEvent::Reopen { .. } = event {
            window().set_visible(true);
            window().set_focus();
        }
    });
}
