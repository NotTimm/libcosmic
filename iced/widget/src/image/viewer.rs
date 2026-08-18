//! Zoom and pan on an image.
use crate::core::border;
use crate::core::image::{self, FilterMethod};
use crate::core::keyboard;
use crate::core::layout;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::touch;
use crate::core::widget::tree::{self, Tree};
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
        }
    }

    /// Sets a message to be produced whenever the [`Viewer`]'s zoom scale changes.
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
        _tree: &mut Tree,
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
            // Mouse-wheel zoom is gated behind Ctrl so plain scrolling never
            // hijacks the wheel; trackpad/touchscreen pinch (below) needs no
            // modifier since the gesture is already unambiguous.
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let state = tree.state.downcast_mut::<State>();
                if !state.modifiers.control() {
                    return;
                }

                let Some(cursor_position) = cursor.position_over(bounds) else {
                    return;
                };

                let y = match *delta {
                    mouse::ScrollDelta::Lines { y, .. }
                    | mouse::ScrollDelta::Pixels { y, .. } => y,
                };

                if y == 0.0 {
                    return;
                }

                let previous_scale = state.scale;
                let new_scale = (if y > 0.0 {
                    state.scale * (1.0 + self.scale_step)
                } else {
                    state.scale / (1.0 + self.scale_step)
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
            // Touchscreen two-finger pinch (distinct from the trackpad
            // gesture above, which arrives as WheelZoomed instead).
            Event::Touch(touch_event) => {
                let state = tree.state.downcast_mut::<State>();

                let id = match *touch_event {
                    touch::Event::FingerPressed { id, .. }
                    | touch::Event::FingerMoved { id, .. }
                    | touch::Event::FingerLifted { id, .. }
                    | touch::Event::FingerLost { id, .. } => id,
                };
                let is_a = state.touch_a.is_some_and(|(i, _)| i == id);
                let is_b = state.touch_b.is_some_and(|(i, _)| i == id);

                match *touch_event {
                    touch::Event::FingerPressed { position, .. } => {
                        if state.touch_a.is_none() {
                            state.touch_a = Some((id, position));
                        } else if state.touch_b.is_none() {
                            state.touch_b = Some((id, position));
                        }
                        state.pinch_last_dist = None;
                    }
                    touch::Event::FingerMoved { position, .. } if is_a || is_b => {
                        if is_a {
                            state.touch_a = Some((id, position));
                        } else {
                            state.touch_b = Some((id, position));
                        }

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
                                        shell.request_redraw();
                                    }
                                }
                            }
                            state.pinch_last_dist = Some(dist);
                            shell.capture_event();
                        }
                    }
                    touch::Event::FingerLifted { .. }
                    | touch::Event::FingerLost { .. } => {
                        if is_a {
                            state.touch_a = None;
                        }
                        if is_b {
                            state.touch_b = None;
                        }
                        state.pinch_last_dist = None;
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(cursor_position) = cursor.position_over(bounds) else {
                    return;
                };

                let state = tree.state.downcast_mut::<State>();

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
                    let hidden_width = (scaled_size.width - bounds.width / 2.0)
                        .max(0.0)
                        .round();

                    let hidden_height = (scaled_size.height
                        - bounds.height / 2.0)
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
        }
    }
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
        let hidden_width =
            (image_size.width - bounds.width / 2.0).max(0.0).round();

        let hidden_height =
            (image_size.height - bounds.height / 2.0).max(0.0).round();

        Vector::new(
            self.current_offset.x.clamp(-hidden_width, hidden_width),
            self.current_offset.y.clamp(-hidden_height, hidden_height),
        )
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
