//! Zoom and pan on an image.
use crate::core::border;
use crate::core::image::{self, FilterMethod};
use crate::core::keyboard;
use crate::core::layout;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::time::{Duration, Instant};
use crate::core::touch;
use crate::core::widget::tree::{self, Tree};
use crate::core::window;
use crate::core::{
    Clipboard, ContentFit, Element, Event, Image, Layout, Length, Pixels,
    Point, Radians, Rectangle, Shell, Size, Vector, Widget,
};

/// A frame that displays an image with the ability to zoom in/out and pan.
pub struct Viewer<'a, Handle, Message> {
    padding: f32,
    width: Length,
    height: Length,
    min_scale: f32,
    max_scale: f32,
    scale_step: f32,
    handle: Handle,
    filter_method: FilterMethod,
    content_fit: ContentFit,
    on_zoom: Option<Box<dyn Fn(f32) -> Message + 'a>>,
    page: Option<u64>,
    whole: bool,
}

impl<'a, Handle, Message> Viewer<'a, Handle, Message> {
    /// Creates a new [`Viewer`] with the given [`State`].
    pub fn new<T: Into<Handle>>(handle: T) -> Self {
        Viewer {
            handle: handle.into(),
            padding: 0.0,
            width: Length::Shrink,
            height: Length::Shrink,
            min_scale: 0.25,
            max_scale: 10.0,
            scale_step: 0.10,
            filter_method: FilterMethod::default(),
            content_fit: ContentFit::default(),
            on_zoom: None,
            page: None,
            whole: false,
        }
    }

    /// Sets a message to be produced whenever the [`Viewer`]'s zoom scale changes.
    /// Identifies the image being shown, e.g. a page number. When it
    /// changes, the zoom is kept but the view moves to the top of the new
    /// image (keeping the horizontal position), as when turning the page of
    /// a zoomed-in comic.
    pub fn page(mut self, page: u64) -> Self {
        self.page = Some(page);
        self
    }

    /// Shows this page whole when it's turned to (zoomed out to fit), as
    /// for a two-page spread. The zoom from before comes back on the next
    /// page that isn't, unless the user zooms in on this one meanwhile.
    pub fn whole_page(mut self, whole: bool) -> Self {
        self.whole = whole;
        self
    }

    /// Sets the message produced when the zoom changes.
    pub fn on_zoom(mut self, f: impl Fn(f32) -> Message + 'a) -> Self {
        self.on_zoom = Some(Box::new(f));
        self
    }

    /// Sets the [`FilterMethod`] of the [`Viewer`].
    pub fn filter_method(mut self, filter_method: image::FilterMethod) -> Self {
        self.filter_method = filter_method;
        self
    }

    /// Sets the [`ContentFit`] of the [`Viewer`].
    pub fn content_fit(mut self, content_fit: ContentFit) -> Self {
        self.content_fit = content_fit;
        self
    }

    /// Sets the padding of the [`Viewer`].
    pub fn padding(mut self, padding: impl Into<Pixels>) -> Self {
        self.padding = padding.into().0;
        self
    }

    /// Sets the width of the [`Viewer`].
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`Viewer`].
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets the max scale applied to the image of the [`Viewer`].
    ///
    /// Default is `10.0`
    pub fn max_scale(mut self, max_scale: f32) -> Self {
        self.max_scale = max_scale;
        self
    }

    /// Sets the min scale applied to the image of the [`Viewer`].
    ///
    /// Default is `0.25`
    pub fn min_scale(mut self, min_scale: f32) -> Self {
        self.min_scale = min_scale;
        self
    }

    /// Sets the percentage the image of the [`Viewer`] will be scaled by
    /// when zoomed in / out.
    ///
    /// Default is `0.10`
    pub fn scale_step(mut self, scale_step: f32) -> Self {
        self.scale_step = scale_step;
        self
    }
}

impl<'a, Message, Theme, Renderer, Handle> Widget<Message, Theme, Renderer>
    for Viewer<'a, Handle, Message>
where
    Renderer: image::Renderer<Handle = Handle>,
    Handle: Clone,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        // The raw w/h of the underlying image
        let image_size =
            renderer.measure_image(&self.handle).unwrap_or_default();

        let image_size =
            Size::new(image_size.width as f32, image_size.height as f32);

        // The size to be available to the widget prior to `Shrink`ing
        let raw_size = limits.resolve(self.width, self.height, image_size);

        // The uncropped size of the image when fit to the bounds above
        let full_size = self.content_fit.fit(image_size, raw_size);

        // Shrink the widget to fit the resized image, if requested
        let final_size = Size {
            width: match self.width {
                Length::Shrink => f32::min(raw_size.width, full_size.width),
                _ => raw_size.width,
            },
            height: match self.height {
                Length::Shrink => f32::min(raw_size.height, full_size.height),
                _ => raw_size.height,
            },
        };

        // A new page: same zoom, but start at its top. A page shown whole
        // (a spread) is zoomed out to fit, and the zoom from before comes
        // back after it.
        let state = tree.state.downcast_mut::<State>();
        if self.page.is_some() && state.page != self.page {
            // A flick on the last page doesn't carry over to this one.
            state.fling = None;
            if state.page.is_some() {
                if self.whole {
                    if state.scale != 1.0 && state.saved_scale.is_none() {
                        state.saved_scale = Some(state.scale);
                    }
                    state.scale = 1.0;
                    state.current_offset = Vector::default();
                } else {
                    if let Some(scale) = state.saved_scale.take() {
                        state.scale = scale;
                    }
                    let scaled = scaled_image_size(renderer, &self.handle, state, final_size, self.content_fit);
                    let hidden_height = ((scaled.height - final_size.height) / 2.0).max(0.0).round();
                    state.current_offset.y = -hidden_height;
                }
                state.starting_offset = state.current_offset;
            }
            state.page = self.page;
        } else if self.whole && state.scale != 1.0 {
            // Zoomed in on the spread: that zoom carries on from here.
            state.saved_scale = None;
        }

        layout::Node::new(final_size)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                let state = tree.state.downcast_mut::<State>();
                state.modifiers = *modifiers;
            }
            // A physical mouse wheel reports discrete `Lines` notches, so it
            // can only ever mean "zoom" here and needs no modifier. A
            // trackpad's two-finger scroll reports continuous `Pixels`; with
            // Ctrl held that also zooms, but without Ctrl it pans instead
            // (free diagonal movement, since x and y move independently —
            // exactly like the two fingers). Panning is a no-op when the
            // image isn't larger than the viewport, since `offset()` clamps
            // it away at draw time.
            //
            // Holding Shift zooms in finer steps. Some systems turn
            // Shift + wheel into a horizontal scroll, so then the x delta
            // counts as the zoom direction.
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let state = tree.state.downcast_mut::<State>();
                state.fling = None;
                let is_physical_wheel =
                    state.wheel.is_physical_wheel(delta, Instant::now());

                if !is_physical_wheel && !state.modifiers.control() {
                    let (x, y) = match *delta {
                        mouse::ScrollDelta::Pixels { x, y } => (x, y),
                        // A Windows touchpad (see `WheelSource`).
                        mouse::ScrollDelta::Lines { x, y } => {
                            (x * PIXELS_PER_LINE, y * PIXELS_PER_LINE)
                        }
                    };

                    if x == 0.0 && y == 0.0 {
                        return;
                    }

                    let _ = pan_by(
                        state,
                        Vector::new(x, y),
                        bounds,
                        renderer,
                        &self.handle,
                        self.content_fit,
                    );

                    shell.request_redraw();
                    shell.capture_event();
                    return;
                }

                let Some(cursor_position) = cursor.position_over(bounds) else {
                    return;
                };

                let fine = state.modifiers.shift();
                let y = match *delta {
                    mouse::ScrollDelta::Lines { x, y }
                    | mouse::ScrollDelta::Pixels { x, y } => {
                        if fine && y == 0.0 { x } else { y }
                    }
                };

                if y == 0.0 {
                    return;
                }

                let step = if fine {
                    self.scale_step / 4.0
                } else {
                    self.scale_step
                };
                let previous_scale = state.scale;
                let new_scale = (if y > 0.0 {
                    state.scale * (1.0 + step)
                } else {
                    state.scale / (1.0 + step)
                })
                .clamp(self.min_scale, self.max_scale);

                if new_scale == previous_scale {
                    return;
                }

                apply_zoom(
                    state,
                    new_scale,
                    cursor_position,
                    bounds,
                    renderer,
                    &self.handle,
                    self.content_fit,
                );

                if let Some(on_zoom) = &self.on_zoom {
                    shell.publish(on_zoom(state.scale));
                }

                shell.request_redraw();
                shell.capture_event();
            }
            // A trackpad/touchscreen pinch gesture. `delta` is already a
            // fractional scale change (positive = magnify).
            Event::Mouse(mouse::Event::WheelZoomed { delta }) => {
                let cursor_position =
                    cursor.position_over(bounds).unwrap_or_else(|| bounds.center());

                let state = tree.state.downcast_mut::<State>();
                let previous_scale = state.scale;
                let new_scale =
                    (state.scale * (1.0 + delta)).clamp(self.min_scale, self.max_scale);

                if new_scale == previous_scale {
                    return;
                }

                apply_zoom(
                    state,
                    new_scale,
                    cursor_position,
                    bounds,
                    renderer,
                    &self.handle,
                    self.content_fit,
                );

                if let Some(on_zoom) = &self.on_zoom {
                    shell.publish(on_zoom(state.scale));
                }

                shell.request_redraw();
                shell.capture_event();
            }
            // Touchscreen: one finger drags a zoomed page around, freely on
            // both axes and 1:1 with the finger, and a flick carries on
            // with momentum. Two fingers pinch to zoom around the point
            // between them and move the page along as they slide (distinct
            // from the trackpad pinch above, which arrives as WheelZoomed).
            Event::Touch(touch_event) => {
                let state = tree.state.downcast_mut::<State>();
                let now = Instant::now();

                let (id, position) = match *touch_event {
                    touch::Event::FingerPressed { id, position }
                    | touch::Event::FingerMoved { id, position }
                    | touch::Event::FingerLifted { id, position }
                    | touch::Event::FingerLost { id, position } => {
                        (id, position)
                    }
                };
                let is_a = state.touch_a.is_some_and(|(i, _)| i == id);
                let is_b = state.touch_b.is_some_and(|(i, _)| i == id);
                state.last_touch = Some(now);

                match *touch_event {
                    touch::Event::FingerPressed { .. } => {
                        if !bounds.contains(position) {
                            return;
                        }
                        // A finger on the page stops a flick and takes over
                        // from a mouse drag Windows may have made of it.
                        state.fling = None;
                        state.cursor_grabbed_at = None;
                        if state.touch_a.is_none() {
                            state.touch_a = Some((id, position));
                        } else if state.touch_b.is_none() {
                            state.touch_b = Some((id, position));
                        }
                        state.reset_touch_gesture();
                    }
                    touch::Event::FingerMoved { .. } if !is_a && !is_b => {
                        // A finger that went down before the viewer was
                        // shown (zoom starts on the second finger of a
                        // pinch): pick it up so the pinch works right away.
                        if !bounds.contains(position) {
                            return;
                        }
                        if state.touch_a.is_none() {
                            state.touch_a = Some((id, position));
                        } else if state.touch_b.is_none() {
                            state.touch_b = Some((id, position));
                        } else {
                            return;
                        }
                        state.fling = None;
                        state.reset_touch_gesture();
                        shell.capture_event();
                    }
                    touch::Event::FingerMoved { .. } => {
                        let slot = if is_a {
                            &mut state.touch_a
                        } else {
                            &mut state.touch_b
                        };
                        let previous = slot.map(|(_, p)| p).unwrap_or(position);
                        *slot = Some((id, position));

                        if let (Some((_, a)), Some((_, b))) =
                            (state.touch_a, state.touch_b)
                        {
                            let dist = ((a.x - b.x).powi(2)
                                + (a.y - b.y).powi(2))
                            .sqrt();
                            let midpoint =
                                Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);

                            if let Some(prev) = state.pinch_last_dist {
                                if prev > 1.0 {
                                    let previous_scale = state.scale;
                                    let new_scale = (state.scale
                                        * (dist / prev))
                                        .clamp(self.min_scale, self.max_scale);

                                    if new_scale != previous_scale {
                                        apply_zoom(
                                            state,
                                            new_scale,
                                            midpoint,
                                            bounds,
                                            renderer,
                                            &self.handle,
                                            self.content_fit,
                                        );

                                        if let Some(on_zoom) = &self.on_zoom {
                                            shell.publish(on_zoom(state.scale));
                                        }
                                    }
                                }
                            }
                            if let Some(last) = state.pinch_last_mid {
                                let _ = pan_by(
                                    state,
                                    midpoint - last,
                                    bounds,
                                    renderer,
                                    &self.handle,
                                    self.content_fit,
                                );
                            }
                            state.pinch_last_dist = Some(dist);
                            state.pinch_last_mid = Some(midpoint);
                        } else {
                            // One finger: the page follows it.
                            let moved = position - previous;
                            let _ = pan_by(
                                state,
                                moved,
                                bounds,
                                renderer,
                                &self.handle,
                                self.content_fit,
                            );
                            state.velocity.track(moved, now);
                        }
                        shell.request_redraw();
                        shell.capture_event();
                    }
                    touch::Event::FingerLifted { .. }
                    | touch::Event::FingerLost { .. } => {
                        if !is_a && !is_b {
                            return;
                        }
                        let alone = state.touch_a.is_none()
                            || state.touch_b.is_none();
                        if alone
                            && matches!(
                                touch_event,
                                touch::Event::FingerLifted { .. }
                            )
                        {
                            if let Some(velocity) = state.velocity.release(now)
                            {
                                state.fling = Some(Fling {
                                    velocity,
                                    last: now,
                                });
                                shell.request_redraw();
                            }
                        }
                        if is_a {
                            state.touch_a = None;
                        }
                        if is_b {
                            state.touch_b = None;
                        }
                        // The finger left on the page carries on from where
                        // it is, without a jump.
                        state.reset_touch_gesture();
                    }
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                let state = tree.state.downcast_mut::<State>();
                let Some(fling) = state.fling.as_mut() else {
                    return;
                };
                let step = fling.step(*now);
                let blocked = pan_by(
                    state,
                    step,
                    bounds,
                    renderer,
                    &self.handle,
                    self.content_fit,
                );
                if let Some(fling) = state.fling.as_mut() {
                    // Hitting an edge stops that direction only.
                    if blocked.0 {
                        fling.velocity.x = 0.0;
                    }
                    if blocked.1 {
                        fling.velocity.y = 0.0;
                    }
                    if fling.is_done() {
                        state.fling = None;
                    } else {
                        shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(cursor_position) = cursor.position_over(bounds) else {
                    return;
                };

                let state = tree.state.downcast_mut::<State>();

                // Windows can also send a finger as a mouse; the touch
                // handling above moves the page already.
                if state.touch_in_use(Instant::now()) {
                    return;
                }

                state.fling = None;
                state.cursor_grabbed_at = Some(cursor_position);
                state.starting_offset = state.current_offset;

                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let state = tree.state.downcast_mut::<State>();

                state.cursor_grabbed_at = None;
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let state = tree.state.downcast_mut::<State>();

                if let Some(origin) = state.cursor_grabbed_at {
                    let scaled_size = scaled_image_size(
                        renderer,
                        &self.handle,
                        state,
                        bounds.size(),
                        self.content_fit,
                    );
                    let hidden_width = ((scaled_size.width - bounds.width) / 2.0)
                        .max(0.0)
                        .round();

                    let hidden_height = ((scaled_size.height
                        - bounds.height)
                        / 2.0)
                        .max(0.0)
                        .round();

                    let delta = *position - origin;

                    let x = if bounds.width < scaled_size.width {
                        (state.starting_offset.x - delta.x)
                            .clamp(-hidden_width, hidden_width)
                    } else {
                        0.0
                    };

                    let y = if bounds.height < scaled_size.height {
                        (state.starting_offset.y - delta.y)
                            .clamp(-hidden_height, hidden_height)
                    } else {
                        0.0
                    };

                    state.current_offset = Vector::new(x, y);
                    shell.request_redraw();
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let is_mouse_over = cursor.is_over(bounds);

        if state.is_cursor_grabbed() {
            mouse::Interaction::Grabbing
        } else if is_mouse_over {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::None
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();

        let final_size = scaled_image_size(
            renderer,
            &self.handle,
            state,
            bounds.size(),
            self.content_fit,
        );

        let translation = {
            let diff_w = bounds.width - final_size.width;
            let diff_h = bounds.height - final_size.height;

            let image_top_left = match self.content_fit {
                ContentFit::None => {
                    Vector::new(diff_w.max(0.0) / 2.0, diff_h.max(0.0) / 2.0)
                }
                _ => Vector::new(diff_w / 2.0, diff_h / 2.0),
            };

            image_top_left - state.offset(bounds, final_size)
        };

        let drawing_bounds = Rectangle::new(bounds.position(), final_size);

        let render = |renderer: &mut Renderer| {
            renderer.with_translation(translation, |renderer| {
                renderer.draw_image(
                    Image {
                        handle: self.handle.clone(),
                        border_radius: border::Radius::default(),
                        filter_method: self.filter_method,
                        rotation: Radians(0.0),
                        opacity: 1.0,
                        snap: true,
                    },
                    drawing_bounds,
                    *viewport - translation,
                );
            });
        };

        renderer.with_layer(bounds, render);
    }
}

/// The local state of a [`Viewer`].
#[derive(Debug, Clone, Copy)]
pub struct State {
    scale: f32,
    starting_offset: Vector,
    current_offset: Vector,
    cursor_grabbed_at: Option<Point>,
    modifiers: keyboard::Modifiers,
    touch_a: Option<(touch::Finger, Point)>,
    touch_b: Option<(touch::Finger, Point)>,
    pinch_last_dist: Option<f32>,
    /// Where the two fingers' midpoint was, to move the page with them.
    pinch_last_mid: Option<Point>,
    /// The last touch event, to ignore the mouse Windows makes of a finger.
    last_touch: Option<Instant>,
    velocity: TouchVelocity,
    fling: Option<Fling>,
    page: Option<u64>,
    /// The zoom to return to after a page shown whole.
    saved_scale: Option<f32>,
    wheel: WheelSource,
}

impl Default for State {
    fn default() -> Self {
        Self {
            scale: 1.0,
            starting_offset: Vector::default(),
            current_offset: Vector::default(),
            cursor_grabbed_at: None,
            modifiers: keyboard::Modifiers::default(),
            touch_a: None,
            touch_b: None,
            pinch_last_dist: None,
            pinch_last_mid: None,
            last_touch: None,
            velocity: TouchVelocity::default(),
            fling: None,
            page: None,
            saved_scale: None,
            wheel: WheelSource::default(),
        }
    }
}

/// Follows a one-finger drag's speed, for the flick when it's let go.
#[derive(Debug, Clone, Copy, Default)]
struct TouchVelocity {
    /// Pixels per second the page is moving (smoothed).
    velocity: Vector,
    last: Option<Instant>,
}

impl TouchVelocity {
    /// Below this a lift just stops the page, as after a slow drag.
    const MIN_FLICK: f32 = 150.0;
    /// So one jumpy event can't send the page flying.
    const MAX_FLICK: f32 = 8000.0;
    /// A finger held still this long before lifting means "stop here".
    const HELD: Duration = Duration::from_millis(80);

    fn track(&mut self, moved: Vector, now: Instant) {
        if let Some(last) = self.last {
            let dt = now.saturating_duration_since(last).as_secs_f32();
            if dt > Self::HELD.as_secs_f32() {
                // Paused mid-drag: start over from here.
                self.velocity = Vector::default();
            } else if dt > 0.0 {
                // Lean on the newest movement, smoothed a little against
                // uneven event timing.
                self.velocity = self.velocity * 0.3 + moved * (0.7 / dt);
            }
        }
        self.last = Some(now);
    }

    /// The flick to carry on with when the finger lifts, if any.
    fn release(&mut self, now: Instant) -> Option<Vector> {
        let held = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) > Self::HELD);
        let v = std::mem::take(self).velocity;
        let speed = (v.x * v.x + v.y * v.y).sqrt();
        if held || speed < Self::MIN_FLICK {
            return None;
        }
        Some(v * (speed.min(Self::MAX_FLICK) / speed))
    }
}

/// A flicked page gliding to a stop.
#[derive(Debug, Clone, Copy)]
struct Fling {
    /// Pixels per second, in the direction the page moves.
    velocity: Vector,
    last: Instant,
}

impl Fling {
    /// How quickly the glide slows: the speed falls to about a third every
    /// this many seconds, and a flick travels its speed times this in all.
    const SLOWDOWN: f32 = 0.35;
    const STOP: f32 = 20.0;

    /// How far the page moves this frame.
    fn step(&mut self, now: Instant) -> Vector {
        let dt = now
            .saturating_duration_since(self.last)
            .as_secs_f32()
            // A frame that came late (window hidden) doesn't jump.
            .min(0.05);
        self.last = now;
        let moved = self.velocity * dt;
        self.velocity = self.velocity * (-dt / Self::SLOWDOWN).exp();
        moved
    }

    fn is_done(&self) -> bool {
        self.velocity.x.abs() < Self::STOP && self.velocity.y.abs() < Self::STOP
    }
}

/// How far a Windows touchpad pans per "line" it reports.
const PIXELS_PER_LINE: f32 = 40.0;

/// Tells a mouse wheel from a touchpad's two-finger scroll.
///
/// Linux (Wayland) and macOS report touchpads in `Pixels` and wheels in
/// `Lines`. Windows reports both as `Lines`: a wheel notch is a whole
/// number of lines (1, or the "lines per notch" setting), while a precision
/// touchpad sends fractions of one. So on Windows fractional lines count
/// as a touchpad, and so does everything else for a moment after them, as
/// a touchpad can land on a whole number mid-gesture.
#[derive(Debug, Clone, Copy, Default)]
pub struct WheelSource {
    touchpad_until: Option<Instant>,
}

impl WheelSource {
    /// How long after fractional lines a scroll still counts as a touchpad.
    const LINGER: Duration = Duration::from_millis(500);

    /// Whether `delta` came from a physical mouse wheel.
    pub fn is_physical_wheel(
        &mut self,
        delta: &mouse::ScrollDelta,
        now: Instant,
    ) -> bool {
        self.classify(delta, now, cfg!(windows))
    }

    fn classify(
        &mut self,
        delta: &mouse::ScrollDelta,
        now: Instant,
        windows: bool,
    ) -> bool {
        let mouse::ScrollDelta::Lines { x, y } = *delta else {
            return false;
        };
        if !windows {
            return true;
        }
        let fractional = |v: f32| (v - v.round()).abs() > 1e-3;
        if fractional(x) || fractional(y) {
            self.touchpad_until = Some(now + Self::LINGER);
            return false;
        }
        !self.touchpad_until.is_some_and(|until| now < until)
    }
}

#[cfg(test)]
mod wheel_tests {
    use super::*;

    fn lines(y: f32) -> mouse::ScrollDelta {
        mouse::ScrollDelta::Lines { x: 0.0, y }
    }

    #[test]
    fn linux_and_mac_keep_lines_as_wheel_and_pixels_as_touchpad() {
        let mut w = WheelSource::default();
        let now = Instant::now();
        assert!(w.classify(&lines(1.0), now, false));
        assert!(w.classify(&lines(0.25), now, false));
        assert!(!w.classify(
            &mouse::ScrollDelta::Pixels { x: 0.0, y: 3.0 },
            now,
            false
        ));
    }

    #[test]
    fn windows_wheel_notches_are_whole_lines() {
        let mut w = WheelSource::default();
        let now = Instant::now();
        assert!(w.classify(&lines(1.0), now, true));
        assert!(w.classify(&lines(-3.0), now, true));
    }

    #[test]
    fn windows_touchpad_sends_fractions_and_lingers() {
        let mut w = WheelSource::default();
        let t0 = Instant::now();
        assert!(!w.classify(&lines(0.075), t0, true));
        assert!(!w.classify(
            &mouse::ScrollDelta::Lines { x: -0.3, y: 0.0 },
            t0,
            true
        ));
        // A whole number mid-gesture is still the touchpad...
        assert!(!w.classify(&lines(1.0), t0 + Duration::from_millis(100), true));
        // ...but a wheel notch well after the gesture zooms again.
        assert!(w.classify(&lines(1.0), t0 + Duration::from_millis(900), true));
    }
}

/// Moves the page on screen by `moved` (as a finger dragging it, or a
/// scroll), clamped right away so it can't be pushed past its edges and
/// moves back at once on reversing. Returns whether x and y hit an edge.
fn pan_by<Renderer, Handle>(
    state: &mut State,
    moved: Vector,
    bounds: Rectangle,
    renderer: &Renderer,
    handle: &Handle,
    content_fit: ContentFit,
) -> (bool, bool)
where
    Renderer: image::Renderer<Handle = Handle>,
{
    let scaled_size =
        scaled_image_size(renderer, handle, state, bounds.size(), content_fit);
    let hidden_width =
        ((scaled_size.width - bounds.width) / 2.0).max(0.0).round();
    let hidden_height =
        ((scaled_size.height - bounds.height) / 2.0).max(0.0).round();

    let unclamped = state.current_offset - moved;
    let x = unclamped.x.clamp(-hidden_width, hidden_width);
    let y = unclamped.y.clamp(-hidden_height, hidden_height);
    state.current_offset = Vector::new(x, y);
    (x != unclamped.x, y != unclamped.y)
}

/// Rescales `state` to `new_scale`, adjusting `current_offset` so the point
/// under `cursor_position` stays fixed on screen (zoom-to-cursor).
fn apply_zoom<Renderer, Handle>(
    state: &mut State,
    new_scale: f32,
    cursor_position: Point,
    bounds: Rectangle,
    renderer: &Renderer,
    handle: &Handle,
    content_fit: ContentFit,
) where
    Renderer: image::Renderer<Handle = Handle>,
{
    let previous_scale = state.scale;
    state.scale = new_scale;

    let scaled_size =
        scaled_image_size(renderer, handle, state, bounds.size(), content_fit);

    let factor = state.scale / previous_scale - 1.0;

    let cursor_to_center = cursor_position - bounds.center();

    let adjustment = cursor_to_center * factor + state.current_offset * factor;

    state.current_offset = Vector::new(
        if scaled_size.width > bounds.width {
            state.current_offset.x + adjustment.x
        } else {
            0.0
        },
        if scaled_size.height > bounds.height {
            state.current_offset.y + adjustment.y
        } else {
            0.0
        },
    );
}

impl State {
    /// Creates a new [`State`].
    pub fn new() -> Self {
        State::default()
    }

    /// Returns the current offset of the [`State`], given the bounds
    /// of the [`Viewer`] and its image.
    fn offset(&self, bounds: Rectangle, image_size: Size) -> Vector {
        // Half the overflow on each axis: at this offset the image's edge
        // lands exactly on the viewport's edge. The previous formula
        // (`size - bounds / 2`) let the image slide well past its own edge
        // into blank space, most noticeably top-to-bottom on tall pages.
        let hidden_width =
            ((image_size.width - bounds.width) / 2.0).max(0.0).round();

        let hidden_height =
            ((image_size.height - bounds.height) / 2.0).max(0.0).round();

        Vector::new(
            self.current_offset.x.clamp(-hidden_width, hidden_width),
            self.current_offset.y.clamp(-hidden_height, hidden_height),
        )
    }

    /// Starts a touch gesture over from the fingers now down: a pinch
    /// measures from where the two fingers are.
    fn reset_touch_gesture(&mut self) {
        (self.pinch_last_dist, self.pinch_last_mid) =
            match (self.touch_a, self.touch_b) {
                (Some((_, a)), Some((_, b))) => (
                    Some(((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()),
                    Some(Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)),
                ),
                _ => (None, None),
            };
        self.velocity = TouchVelocity::default();
    }

    /// Whether a finger is down or just lifted.
    fn touch_in_use(&self, now: Instant) -> bool {
        self.touch_a.is_some()
            || self.touch_b.is_some()
            || self.last_touch.is_some_and(|t| {
                now.saturating_duration_since(t) < Duration::from_millis(500)
            })
    }

    /// Returns if the cursor is currently grabbed by the [`Viewer`].
    pub fn is_cursor_grabbed(&self) -> bool {
        self.cursor_grabbed_at.is_some()
    }
}

impl<'a, Message, Theme, Renderer, Handle> From<Viewer<'a, Handle, Message>>
    for Element<'a, Message, Theme, Renderer>
where
    Renderer: 'a + image::Renderer<Handle = Handle>,
    Message: 'a,
    Handle: Clone + 'a,
{
    fn from(viewer: Viewer<'a, Handle, Message>) -> Element<'a, Message, Theme, Renderer> {
        Element::new(viewer)
    }
}

/// Returns the bounds of the underlying image, given the bounds of
/// the [`Viewer`]. Scaling will be applied and original aspect ratio
/// will be respected.
pub fn scaled_image_size<Renderer>(
    renderer: &Renderer,
    handle: &<Renderer as image::Renderer>::Handle,
    state: &State,
    bounds: Size,
    content_fit: ContentFit,
) -> Size
where
    Renderer: image::Renderer,
{
    let Size { width, height } =
        renderer.measure_image(handle).unwrap_or_default();

    let image_size = Size::new(width as f32, height as f32);

    let adjusted_fit = content_fit.fit(image_size, bounds);

    Size::new(
        adjusted_fit.width * state.scale,
        adjusted_fit.height * state.scale,
    )
}

#[cfg(test)]
mod touch_tests {
    use super::*;
    use crate::core::clipboard;

    /// A 100×100 viewer on the null renderer (whose images measure
    /// 100×100), zoomed to 2×: 50 px of page hidden on every side.
    struct Harness {
        viewer: Viewer<'static, image::Handle, ()>,
        tree: Tree,
        node: layout::Node,
    }

    impl Harness {
        fn new() -> Self {
            let mut state = State::default();
            state.scale = 2.0;
            Harness {
                viewer: Viewer::new(image::Handle::from_rgba(1, 1, vec![0; 4]))
                    .width(Length::Fill)
                    .height(Length::Fill),
                tree: Tree {
                    tag: tree::Tag::of::<State>(),
                    id: None,
                    state: tree::State::new(state),
                    children: Vec::new(),
                },
                node: layout::Node::new(Size::new(100.0, 100.0)),
            }
        }

        /// Sends an event; returns whether it was captured.
        fn send(&mut self, event: Event) -> bool {
            let mut messages = Vec::new();
            let mut shell = Shell::new(&mut messages);
            Widget::<(), (), ()>::update(
                &mut self.viewer,
                &mut self.tree,
                &event,
                Layout::new(&self.node),
                mouse::Cursor::Unavailable,
                &(),
                &mut clipboard::Null,
                &mut shell,
                &Rectangle::new(Point::ORIGIN, Size::new(100.0, 100.0)),
            );
            shell.is_event_captured()
        }

        fn finger(&mut self, f: fn(touch::Finger, Point) -> touch::Event, id: u64, x: f32, y: f32) -> bool {
            self.send(Event::Touch(f(touch::Finger(id), Point::new(x, y))))
        }

        fn state(&mut self) -> &mut State {
            self.tree.state.downcast_mut::<State>()
        }

        fn offset(&mut self) -> Vector {
            self.state().current_offset
        }
    }

    fn pressed(id: touch::Finger, position: Point) -> touch::Event {
        touch::Event::FingerPressed { id, position }
    }
    fn moved(id: touch::Finger, position: Point) -> touch::Event {
        touch::Event::FingerMoved { id, position }
    }
    fn lifted(id: touch::Finger, position: Point) -> touch::Event {
        touch::Event::FingerLifted { id, position }
    }

    #[test]
    fn one_finger_drags_both_axes_with_the_finger() {
        let mut h = Harness::new();
        let _ = h.finger(pressed, 1, 50.0, 50.0);
        // Diagonal: the page follows the finger on both axes at once.
        assert!(h.finger(moved, 1, 40.0, 30.0));
        // Same direction as dragging with the mouse.
        assert_eq!(h.offset(), Vector::new(10.0, 20.0));
        let _ = h.finger(moved, 1, 45.0, 35.0);
        assert_eq!(h.offset(), Vector::new(5.0, 15.0));
    }

    #[test]
    fn drag_stops_at_the_edge_and_comes_back_at_once() {
        let mut h = Harness::new();
        let _ = h.finger(pressed, 1, 90.0, 50.0);
        let _ = h.finger(moved, 1, 10.0, 50.0);
        assert_eq!(h.offset(), Vector::new(50.0, 0.0));
        // Reversing moves right away, with no slack to unwind.
        let _ = h.finger(moved, 1, 15.0, 50.0);
        assert_eq!(h.offset(), Vector::new(45.0, 0.0));
    }

    #[test]
    fn a_page_that_fits_does_not_move() {
        let mut h = Harness::new();
        h.state().scale = 1.0;
        let _ = h.finger(pressed, 1, 50.0, 50.0);
        let _ = h.finger(moved, 1, 20.0, 20.0);
        assert_eq!(h.offset(), Vector::new(0.0, 0.0));
    }

    #[test]
    fn a_press_outside_the_page_is_ignored() {
        let mut h = Harness::new();
        let _ = h.finger(pressed, 1, 50.0, 150.0);
        assert!(!h.finger(moved, 1, 40.0, 140.0));
        assert_eq!(h.offset(), Vector::new(0.0, 0.0));
    }

    #[test]
    fn two_fingers_move_the_page_with_them() {
        let mut h = Harness::new();
        let _ = h.finger(pressed, 1, 30.0, 50.0);
        let _ = h.finger(pressed, 2, 70.0, 50.0);
        // Both slide up by 10 without spreading: no zoom, page moves up.
        let _ = h.finger(moved, 1, 30.0, 40.0);
        let _ = h.finger(moved, 2, 70.0, 40.0);
        // (One finger at a time, so the spread wobbles and settles back.)
        assert!((h.state().scale - 2.0).abs() < 1e-4);
        assert!((h.offset().y - 10.0).abs() < 0.5, "{:?}", h.offset());
    }

    #[test]
    fn pinch_zooms_and_lifting_one_finger_carries_on_without_a_jump() {
        let mut h = Harness::new();
        let _ = h.finger(pressed, 1, 40.0, 50.0);
        let _ = h.finger(pressed, 2, 60.0, 50.0);
        let _ = h.finger(moved, 1, 40.0, 50.0);
        let _ = h.finger(moved, 2, 80.0, 50.0);
        assert!(h.state().scale > 2.0);
        let _ = h.finger(lifted, 2, 80.0, 50.0);
        let before = h.offset();
        let _ = h.finger(moved, 1, 35.0, 50.0);
        assert_eq!(h.offset(), before + Vector::new(5.0, 0.0));
    }

    #[test]
    fn fingers_down_before_the_viewer_appeared_still_pinch() {
        let mut h = Harness::new();
        // No presses seen: zoom mode began on the second finger.
        let _ = h.finger(moved, 1, 40.0, 50.0);
        let _ = h.finger(moved, 2, 60.0, 50.0);
        let _ = h.finger(moved, 2, 80.0, 50.0);
        assert!(h.state().scale > 2.0);
    }

    #[test]
    fn a_mouse_press_from_a_finger_does_not_drag_too() {
        let mut h = Harness::new();
        let _ = h.finger(pressed, 1, 50.0, 50.0);
        let _ = h.send(Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)));
        assert!(!h.state().is_cursor_grabbed());
    }

    #[test]
    fn a_flick_glides_on_and_stops() {
        let mut h = Harness::new();
        h.state().scale = 10.0; // plenty of room to glide
        let t0 = Instant::now();
        h.state().fling = Some(Fling { velocity: Vector::new(-1000.0, 0.0), last: t0 });
        let mut t = t0;
        let mut frames = 0;
        while h.state().fling.is_some() {
            t += Duration::from_millis(16);
            let _ = h.send(Event::Window(window::Event::RedrawRequested(t)));
            frames += 1;
            assert!(frames < 1000);
        }
        // About speed × SLOWDOWN in all (1000 × 0.35), and done in a couple
        // of seconds.
        let x = h.offset().x;
        assert!((300.0..360.0).contains(&x), "{x}");
        assert!(t - t0 < Duration::from_secs(3));
    }

    #[test]
    fn a_flick_stops_at_the_edge() {
        let mut h = Harness::new();
        let t0 = Instant::now();
        h.state().fling = Some(Fling { velocity: Vector::new(-5000.0, 0.0), last: t0 });
        let mut t = t0;
        for _ in 0..20 {
            t += Duration::from_millis(16);
            let _ = h.send(Event::Window(window::Event::RedrawRequested(t)));
        }
        assert_eq!(h.offset().x, 50.0);
        assert!(h.state().fling.is_none());
    }

    #[test]
    fn velocity_is_kept_for_a_quick_flick_only() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);

        let mut v = TouchVelocity::default();
        for i in 1..=5 {
            v.track(Vector::new(0.0, -16.0), ms(i * 16));
        }
        let flick = v.release(ms(85)).expect("a flick");
        assert!(flick.y < -800.0 && flick.x == 0.0, "{flick:?}");

        // Held still before lifting: stays put.
        let mut v = TouchVelocity::default();
        for i in 1..=5 {
            v.track(Vector::new(0.0, -16.0), ms(i * 16));
        }
        assert!(v.release(ms(300)).is_none());

        // A slow drag doesn't glide.
        let mut v = TouchVelocity::default();
        for i in 1..=5 {
            v.track(Vector::new(1.0, 0.0), ms(i * 16));
        }
        assert!(v.release(ms(85)).is_none());
    }
}
