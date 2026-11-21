use super::*;

/// How long a window's snapshot covers its page while its page hasn't heard back from what it first asked, so the page
/// shows with what it asked for, or as it is, as its page turns out to need.
pub(super) const SHOWN_WITHIN: std::time::Duration = std::time::Duration::from_millis(1000);
/// How long a window with no snapshot shows its bare canvas while its page starts, before the page shows as it is,
/// its own loading state, so a quick page shows no spinner (Andre, 20 Nov), and how long what covers a page takes to
/// fade into it.
pub(super) const QUIET: std::time::Duration = std::time::Duration::from_millis(150);
/// How long after it opens a window that shows nothing of its page is blank (`Event::Blank`): long past a slow page's
/// start (the Surface's Settings, 2 to 3 s), where its person is looking at an empty window.
pub(super) const BLANK: std::time::Duration = std::time::Duration::from_secs(5);
pub(super) const FADES_IN: std::time::Duration = std::time::Duration::from_millis(200);

/// What covers a window's page as it opens, as iOS shows an app's snapshot as it launches: its snapshot (`picture`),
/// or with none its canvas (the page's background, under its title row), from the moment it shows until its page is
/// ready, made as it's first drawn; then fading into it. A page shown still starting, its own loading state, fades
/// into what it shows once it's ready the same way, what was on the screen its cover.
pub(super) enum Cover {
    Over(Option<crate::render::scene::Scene>),
    Fading(crate::render::scene::Scene, web_time::Instant),
    Gone,
}

impl Cover {
    /// How opaque it's drawn now, easing in and out as it fades.
    pub(super) fn opacity(&self) -> f32 {
        match self {
            Cover::Over(_) => 1.0,
            Cover::Fading(_, began) => {
                let t = (since(*began).as_secs_f32() / FADES_IN.as_secs_f32()).min(1.0);
                1.0 - t * t * (3.0 - 2.0 * t)
            }
            Cover::Gone => 0.0,
        }
    }

    pub(super) fn scene(&self) -> Option<&crate::render::scene::Scene> {
        match self {
            Cover::Over(scene) => scene.as_ref(),
            Cover::Fading(scene, _) => Some(scene),
            Cover::Gone => None,
        }
    }
}

/// The whole of a window `size` device pixels big.
pub(super) fn whole(size: Size<DevicePixels>) -> crate::geometry::Bounds<crate::geometry::ScaledPixels> {
    let px = crate::geometry::ScaledPixels;
    crate::geometry::Bounds { origin: Point { x: px(0.0), y: px(0.0) }, size: Size { width: px(size.width.0 as f32), height: px(size.height.0 as f32) } }
}

/// What a window shows: its page's scene, with `cover` over it `opacity` as opaque, in a scene of its own, so its
/// page's is as its page painted it, which it paints on from.
pub(super) fn covered(page: &crate::render::scene::Scene, cover: Option<&crate::render::scene::Scene>, opacity: f32, size: Size<DevicePixels>) -> crate::render::scene::Scene {
    let mut shown = crate::render::scene::Scene::default();
    shown.splice(page, 1.0, [0.0, 0.0], 1.0, whole(size));
    if let Some(cover) = cover {
        shown.splice(cover, 1.0, [0.0, 0.0], opacity, whole(size));
    }
    shown.finish();
    shown
}
