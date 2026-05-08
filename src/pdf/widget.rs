use std::{
    cell::RefCell,
    collections::{HashMap},
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};

use anyhow::Result;
use colorgrad::{Gradient as _, GradientBuilder, LinearGradient};
use iced::{
    Renderer, Size,
    advanced::{Renderer as _, graphics::geometry, image},
    widget::{
        self,
        canvas::{self, Cache, Stroke},
    },
};
use mupdf::{Colorspace, Device, Matrix, Pixmap};
use tracing::debug;

use crate::{
    DARK_THEME,
    config::{MOVE_STEP, MouseAction},
    geometry::{Rect, Vector},
    pdf::{PdfMessage, outline_extraction::OutlineItem, page_layout::PageLayout},
};

const MIN_SELECTION: f32 = 5.0;
const MIN_CLICK_DISTANCE: f32 = 5.0;

/// `mupdf::Pixmap` is `Send` because it owns its own pixel buffer and is only
/// accessed immutably here, so an `Arc` around it satisfies the `Send + 'static`
/// bounds required by `iced::advanced::image::Bytes`.
#[derive(Debug)]
struct PixmapBytes(Arc<Pixmap>);

// Safety: `Pixmap` owns its pixel buffer and we only access it immutably
// (`samples()`). The underlying `*mut fz_pixmap` is never mutated after
// the pixmap has been rendered, so it is safe to send the owned buffer
// to another thread (e.g. the GPU upload thread used by iced).
unsafe impl Send for PixmapBytes {}
unsafe impl Sync for PixmapBytes {}

impl AsRef<[u8]> for PixmapBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.samples()
    }
}

/// A pixel buffer that returns itself to a shared pool when dropped.
#[derive(Debug)]
struct PooledBuffer {
    buf: Option<Vec<u8>>,
    pool: Weak<Mutex<HashMap<usize, Vec<Vec<u8>>>>>,
    page_idx: usize,
}

impl AsRef<[u8]> for PooledBuffer {
    fn as_ref(&self) -> &[u8] {
        self.buf.as_ref().expect("Buffer should not be None")
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let Some(buf) = self.buf.take() {
            if let Some(pool) = self.pool.upgrade() {
                if let Ok(mut pool) = pool.lock() {
                    pool.entry(self.page_idx).or_default().push(buf);
                }
            }
        }
    }
}

type BufferPool = Arc<Mutex<HashMap<usize, Vec<Vec<u8>>>>>;

/// Cache key for rendered page images.
///
/// * `Full` is used when the entire page fits inside the viewport. The cached
///   image is independent of translation so panning does not trigger re-renders.
/// * `Partial` is used when only a sub-rect of the page is visible. The key
///   includes the visible rectangle (in viewport pixels) so that any pan or
///   zoom invalidates the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RenderKey {
    Full(usize, u32),
    Partial(usize, u32, i32, i32, i32, i32),
}

struct Document {
    cache: Cache,
    pages: Vec<(image::Handle, Rect<f32>)>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("cache", &self.cache)
            .field("page_count", &self.pages.len())
            .finish()
    }
}

impl Document {
    pub fn new(pages: Vec<(image::Handle, Rect<f32>)>) -> Self {
        Self {
            cache: Cache::default(),
            pages,
        }
    }
}

impl<'a> widget::canvas::Program<PdfMessage> for Document {
    type State = ();

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        theme: &iced::Theme,
        bounds: iced::Rectangle,
        cursor: iced::advanced::mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let _span = tracy_client::span!("Pdf draw");
        let bg = self.cache.draw(renderer, bounds.size(), |frame| {
            for (handle, rect) in &self.pages {
                let mut c = iced::Color::WHITE;
                c.a = 0.2;
                frame.fill_rectangle((rect.x0).into(), rect.size().into(), c);
                frame.stroke_rectangle(
                    (rect.x0).into(),
                    rect.size().into(),
                    Stroke::default().with_color(c).with_width(1.0),
                );
                frame.fill_rectangle(
                    (rect.center() - Vector::new(2.0, 2.0)).into(),
                    iced::Size::new(4.0, 4.0),
                    iced::Color::from_rgb(0.0, 1.0, 0.0),
                );
                frame.fill_text(geometry::Text {
                    content: format!("({}, {})", rect.x0.x, rect.x0.y),
                    position: rect.x0.into(),
                    color: iced::Color::from_rgb(0.0, 1.0, 0.0),
                    size: 12.0.into(),
                    line_height: widget::text::LineHeight::Relative(1.0),
                    font: iced::Font::default(),
                    horizontal_alignment: iced::alignment::Horizontal::Left,
                    vertical_alignment: iced::alignment::Vertical::Top,
                    shaping: widget::text::Shaping::Basic,
                });
                frame.fill_text(geometry::Text {
                    content: format!("({}, {})", rect.x1.x, rect.x0.y),
                    position: Vector::new(rect.x1.x, rect.x0.y).into(),
                    color: iced::Color::from_rgb(0.0, 1.0, 0.0),
                    size: 12.0.into(),
                    line_height: widget::text::LineHeight::Relative(1.0),
                    font: iced::Font::default(),
                    horizontal_alignment: iced::alignment::Horizontal::Right,
                    vertical_alignment: iced::alignment::Vertical::Top,
                    shaping: widget::text::Shaping::Basic,
                });
                frame.fill_text(geometry::Text {
                    content: format!("({}, {})", rect.x0.x, rect.x1.y),
                    position: Vector::new(rect.x0.x, rect.x1.y).into(),
                    color: iced::Color::from_rgb(0.0, 1.0, 0.0),
                    size: 12.0.into(),
                    line_height: widget::text::LineHeight::Relative(1.0),
                    font: iced::Font::default(),
                    horizontal_alignment: iced::alignment::Horizontal::Left,
                    vertical_alignment: iced::alignment::Vertical::Bottom,
                    shaping: widget::text::Shaping::Basic,
                });
                frame.fill_text(geometry::Text {
                    content: format!("({}, {})", rect.x1.x, rect.x1.y),
                    position: rect.x1.into(),
                    color: iced::Color::from_rgb(0.0, 1.0, 0.0),
                    size: 12.0.into(),
                    line_height: widget::text::LineHeight::Relative(1.0),
                    font: iced::Font::default(),
                    horizontal_alignment: iced::alignment::Horizontal::Right,
                    vertical_alignment: iced::alignment::Vertical::Bottom,
                    shaping: widget::text::Shaping::Basic,
                });
                frame.fill_text(geometry::Text {
                    content: format!("({}, {})", rect.center().x, rect.center().y),
                    position: rect.center().into(),
                    color: iced::Color::from_rgb(0.0, 1.0, 0.0),
                    size: 12.0.into(),
                    line_height: widget::text::LineHeight::Relative(1.0),
                    font: iced::Font::default(),
                    horizontal_alignment: iced::alignment::Horizontal::Left,
                    vertical_alignment: iced::alignment::Vertical::Top,
                    shaping: widget::text::Shaping::Basic,
                });

                let bounds: iced::Rectangle = (*rect).into();
                frame.draw_image(bounds, handle);
            }
            let bounds_size = Vector::new(bounds.size().width, bounds.size().height).scaled(0.5);
            frame.fill_rectangle(
                (bounds_size - Vector::new(2.0, 2.0)).into(),
                iced::Size::new(4.0, 4.0),
                iced::Color::from_rgb(1.0, 0.0, 0.0),
            );
        });
        vec![bg]
    }
}

#[derive(Debug)]
pub enum MouseInteraction {
    None,
    Panning,
    Selecting,
}

/// A pixmap is cached by its page number and the zoom level at which it was generated.
/// Renders a pdf document. Owns all information related to the document.
#[derive(Debug)]
pub struct PdfViewer {
    pub name: String,
    pub path: PathBuf,

    pub invert_colors: bool,
    pub draw_page_borders: bool,

    doc: mupdf::Document,
    display_lists: Vec<mupdf::DisplayList>,
    /// Exact render cache: key includes visible rect for partial renders.
    render_cache: RefCell<HashMap<RenderKey, image::Handle>>,
    /// Reusable mupdf pixmaps (one per page). Only the most recent size is kept.
    pixmap_pool: RefCell<HashMap<usize, Pixmap>>,
    /// Shared pool of CPU buffers returned by dropped images.
    buffer_pool: BufferPool,

    pub translation: Vector<f32>,
    pub scale: f32,
    fractional_scaling: f32,

    viewport: RefCell<Size<f32>>,

    mouse_pos: Vector<f32>,
    mouse_pressed_at: Vector<f32>,
    mouse_interaction: MouseInteraction,

    layout: PageLayout,

    gradient_cache: [[u8; 4]; 256],
}

impl PdfViewer {
    pub fn from_path(path: PathBuf) -> Result<Self> {
        let name = path
            .file_name()
            .expect("The pdf must have a file name")
            .to_string_lossy()
            .to_string();
        let doc = mupdf::Document::open(&path.to_str().unwrap())?;
        let mut display_lists = vec![];
        for page in doc.pages()?.flatten() {
            let dl = mupdf::DisplayList::new(page.bounds()?)?;
            let dummy_device = Device::from_display_list(&dl)?;
            let ctm = Matrix::IDENTITY;
            page.run(&dummy_device, &ctm)?;
            display_lists.push(dl);
        }

        let bg_color = DARK_THEME
            .extended_palette()
            .background
            .base
            .color
            .into_rgba8();
        let mut gradient_cache = [[0; 4]; 256];
        generate_gradient_cache(&mut gradient_cache, &bg_color);

        Ok(PdfViewer {
            name,
            path,
            invert_colors: false,
            draw_page_borders: true,
            doc,
            display_lists,
            render_cache: RefCell::default(),
            pixmap_pool: RefCell::default(),
            buffer_pool: Arc::new(Mutex::new(HashMap::new())),
            translation: Vector::zero(),
            scale: 1.0,
            fractional_scaling: 1.0,
            viewport: RefCell::default(),
            layout: PageLayout::SinglePage,
            gradient_cache,
            mouse_pos: Vector::zero(),
            mouse_pressed_at: Vector::zero(),
            mouse_interaction: MouseInteraction::None,
        })
    }

    pub fn update(&mut self, msg: PdfMessage) -> iced::Task<PdfMessage> {
        let mut out = iced::Task::none();
        let page_count = self.doc.page_count().unwrap() as usize;
        match msg {
            PdfMessage::PageDown => {
                let current = self
                    .layout
                    .center_of_page(&self.doc, self.translation, self.viewport.borrow().clone())
                    .unwrap();
                let next = self
                    .layout
                    .center_of_page_below(
                        &self.doc,
                        self.translation,
                        self.viewport.borrow().clone(),
                    )
                    .unwrap();

                self.translation.y += next.center().y - current.center().y;
            }
            PdfMessage::PageUp => {
                let current = self
                    .layout
                    .center_of_page(&self.doc, self.translation, self.viewport.borrow().clone())
                    .unwrap();
                let prev = self
                    .layout
                    .center_of_page_above(
                        &self.doc,
                        self.translation,
                        self.viewport.borrow().clone(),
                    )
                    .unwrap();

                self.translation.y += prev.center().y - current.center().y;
            }
            PdfMessage::SetPage(idx) => {
                if idx < page_count && idx > 0 && page_count > 0 {
                    if let Ok(translation) = self.layout.translation_for_page(
                        &self.doc,
                        self.scale,
                        self.fractional_scaling,
                        idx,
                        self.viewport.borrow().clone(),
                    ) {
                        self.translation = translation;
                    }
                }
            }
            PdfMessage::SetTranslation(vector) => {
                self.translation = vector;
            }
            PdfMessage::SetLocation(vector, scale) => {
                self.translation = vector;
                self.scale = scale;
            }
            PdfMessage::SetLayout(page_layout) => {
                self.layout = page_layout;
            }
            PdfMessage::ZoomIn => {
                self.scale *= 1.2;
            }
            PdfMessage::ZoomOut => {
                self.scale /= 1.2;
            }
            PdfMessage::ZoomHome => {
                self.scale = 1.0;
            }
            PdfMessage::ZoomFit => {
                self.translation = Vector::zero();
            }
            PdfMessage::Move(vector) => {
                self.translation += vector;
            }
            PdfMessage::MouseMoved(vector) => {
                match self.mouse_interaction {
                    MouseInteraction::None => {}
                    MouseInteraction::Panning => {
                        out = iced::Task::done(PdfMessage::Move(
                            (self.mouse_pos - vector)
                                .scaled(1.0 / (self.scale * self.fractional_scaling)),
                        ))
                    }
                    MouseInteraction::Selecting => todo!(),
                }
                self.mouse_pos = vector;
            }
            PdfMessage::MouseAction(mouse_action, pressed) => {
                if pressed {
                    match mouse_action {
                        MouseAction::Panning => {
                            self.mouse_interaction = MouseInteraction::Panning;
                            self.mouse_pressed_at = self.mouse_pos;
                        }
                        MouseAction::Selection => {
                            self.mouse_interaction = MouseInteraction::Selecting;
                            self.mouse_pressed_at = self.mouse_pos;
                        }
                        MouseAction::NextPage => {
                            out = iced::Task::done(PdfMessage::PageDown);
                        }
                        MouseAction::PreviousPage => {
                            out = iced::Task::done(PdfMessage::PageUp);
                        }
                        MouseAction::ZoomIn => {
                            out = iced::Task::done(PdfMessage::ZoomIn);
                        }
                        MouseAction::ZoomOut => {
                            out = iced::Task::done(PdfMessage::ZoomOut);
                        }
                        MouseAction::MoveUp => {
                            out = iced::Task::done(PdfMessage::Move(Vector::new(0.0, -MOVE_STEP)));
                        }
                        MouseAction::MoveDown => {
                            out = iced::Task::done(PdfMessage::Move(Vector::new(0.0, MOVE_STEP)));
                        }
                        MouseAction::MoveLeft => {
                            out = iced::Task::done(PdfMessage::Move(Vector::new(-MOVE_STEP, 0.0)));
                        }
                        MouseAction::MoveRight => {
                            out = iced::Task::done(PdfMessage::Move(Vector::new(MOVE_STEP, 0.0)));
                        }
                    }
                } else {
                    match self.mouse_interaction {
                        MouseInteraction::None | MouseInteraction::Panning => {}
                        MouseInteraction::Selecting => {
                            // TODO: Copy text
                        }
                    }
                    self.mouse_interaction = MouseInteraction::None;
                }
            }
            PdfMessage::ToggleLinkHitboxes => {}
            PdfMessage::ActivateLink(_) => {}
            PdfMessage::CloseLinkHitboxes => {}
            PdfMessage::FileChanged => {}
            PdfMessage::PrintPdf => {}
            PdfMessage::None => {}
        }
        out
    }

    pub fn view(&self) -> iced::Element<'_, PdfMessage> {
        widget::responsive(|size| {
            {
                let mut viewport = self.viewport.borrow_mut();
                *viewport = size;
            }
            let rects = self
                .layout
                .pages_rects(
                    &self.doc,
                    self.translation.scaled(-1.0),
                    self.scale,
                    self.fractional_scaling,
                    size,
                )
                .unwrap();
            let viewport_rect =
                Rect::from_pos_size(Vector::zero(), Vector::new(size.width, size.height));

            let effective_scale = self.scale * self.fractional_scaling;

            // Drop pixmap allocations for pages that are no longer visible.
            let visible_indices: Vec<usize> = rects
                .iter()
                .enumerate()
                .filter(|(_, r)| viewport_rect.intersects(r))
                .map(|(i, _)| i)
                .collect();
            self.pixmap_pool
                .borrow_mut()
                .retain(|idx, _| visible_indices.contains(idx));

            let with_handles: Vec<_> = rects
                .into_iter()
                .zip(self.doc.pages().unwrap())
                .enumerate()
                .filter(|(_, (r, _page))| viewport_rect.intersects(r))
                .map(|(i, (rect_ss, page))| {
                    // rect_ss = A pages bounding box in screen coordinates (relative to the widgets origin)
                    let page = page.unwrap();
                    let page_bounds: Rect<f32> = page.bounds().unwrap().into();

                    let fully_visible = rect_ss.x0.x >= 0.0
                        && rect_ss.x1.x <= viewport_rect.x1.x
                        && rect_ss.x0.y >= 0.0
                        && rect_ss.x1.y <= viewport_rect.x1.y;

                    let (key, draw_rect, w, h, matrix, scissor) = if fully_visible {
                        let key = RenderKey::Full(i, effective_scale.to_bits());
                        let w = rect_ss.width().ceil().max(1.0) as i32;
                        let h = rect_ss.height().ceil().max(1.0) as i32;
                        let tx = -page_bounds.x0.x * effective_scale;
                        let ty = -page_bounds.x0.y * effective_scale;
                        let matrix =
                            Matrix::new(effective_scale, 0.0, 0.0, effective_scale, tx, ty);
                        let scissor = mupdf::Rect::new(0.0, 0.0, w as f32, h as f32);
                        (key, rect_ss, w, h, matrix, scissor)
                    } else {
                        let vis = rect_ss.intersect(&viewport_rect);
                        let vw = vis.width().ceil().max(1.0) as i32;
                        let vh = vis.height().ceil().max(1.0) as i32;

                        let render_offset_x = rect_ss.x0.x - vis.x0.x;
                        let render_offset_y = rect_ss.x0.y - vis.x0.y;

                        // Round to integer pixels for the cache key. The
                        // matrix uses the snapped value; the draw position is
                        // adjusted by the rounding error so they stay in sync.
                        let snapped_offset_x = render_offset_x.round();
                        let snapped_offset_y = render_offset_y.round();

                        let key = RenderKey::Partial(
                            i,
                            effective_scale.to_bits(),
                            snapped_offset_x as i32,
                            snapped_offset_y as i32,
                            vw,
                            vh,
                        );

                        let raster_tx = snapped_offset_x - page_bounds.x0.x * effective_scale;
                        let raster_ty = snapped_offset_y - page_bounds.x0.y * effective_scale;
                        let matrix = Matrix::new(
                            effective_scale,
                            0.0,
                            0.0,
                            effective_scale,
                            raster_tx,
                            raster_ty,
                        );
                        let scissor = mupdf::Rect::new(0.0, 0.0, vw as f32, vh as f32);

                        // Compensate for snapping so the image is drawn at the
                        // correct sub-pixel position. draw_x = r.x0.x - snapped_offset_x
                        // which is the rounding error in [-0.5, 0.5].
                        let draw_rect = Rect::from_pos_size(
                            Vector::new(
                                rect_ss.x0.x - snapped_offset_x,
                                rect_ss.x0.y - snapped_offset_y,
                            ),
                            Vector::new(vw as f32, vh as f32),
                        );

                        (key, draw_rect, vw, vh, matrix, scissor)
                    };

                    let mut cache = self.render_cache.borrow_mut();
                    if !cache.contains_key(&key) {
                        let _span = tracy_client::span!("Pdf cache miss");
                        debug!("Cache miss for page {}", i);

                        // Try to reuse a pixmap allocation for this page.
                        let mut pool = self.pixmap_pool.borrow_mut();
                        let mut pix = pool.remove(&i).unwrap_or_else(|| {
                            Pixmap::new_with_w_h(&Colorspace::device_rgb(), w, h, true).unwrap()
                        });

                        // If the pooled pixmap has the wrong size, allocate a new one.
                        if pix.width() as i32 != w || pix.height() as i32 != h {
                            pix = Pixmap::new_with_w_h(&Colorspace::device_rgb(), w, h, true)
                                .unwrap();
                        }

                        pix.samples_mut().fill(255);
                        let device = Device::from_pixmap(&mut pix).unwrap();
                        self.display_lists[i]
                            .run(&device, &matrix, scissor)
                            .unwrap();

                        if self.invert_colors {
                            cpu_pdf_dark_mode_shader(&mut pix, &self.gradient_cache);
                        }

                        // TODO: This is NOT zero-copy. Can we make it?
                        let samples = pix.samples();

                        // Try to reuse a CPU buffer from the shared pool.
                        let mut buf = self
                            .buffer_pool
                            .lock()
                            .unwrap()
                            .remove(&i)
                            .and_then(|mut v| v.pop())
                            .unwrap_or_else(|| Vec::with_capacity(samples.len()));
                        // PERF: if samples.len() > buf.capacity() this results in a re-allocation
                        buf.clear();
                        buf.extend_from_slice(samples);
                        // Return the mupdf pixmap to the pool for reuse.
                        pool.insert(i, pix);

                        let handle = image::Handle::from_rgba(
                            w as u32,
                            h as u32,
                            image::Bytes::from_owner(PooledBuffer {
                                buf: Some(buf),
                                pool: Arc::downgrade(&self.buffer_pool),
                                page_idx: i,
                            }),
                        );
                        cache.insert(key, handle);
                    }
                    let handle = cache.get(&key).unwrap().clone();
                    (handle, draw_rect)
                })
                .collect();

            widget::canvas(Document::new(with_handles))
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into()
        })
        .into()
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.fractional_scaling = scale_factor as f32;
    }

    pub fn is_jumpable_action(&self, msg: &PdfMessage) -> bool {
        // TODO: Implement
        false
    }

    pub fn get_outline(&self) -> &[OutlineItem] {
        // TODO: Implement
        &[]
    }

    pub fn page_progress(&self) -> &str {
        // TODO: Implement
        "(? / ?)"
    }
}

fn generate_gradient_cache(cache: &mut [[u8; 4]; 256], bg_color: &[u8; 4]) {
    let gradient = GradientBuilder::new()
        .colors(&[
            colorgrad::Color::from_rgba8(255, 255, 255, 255),
            colorgrad::Color::from_rgba8(bg_color[0], bg_color[1], bg_color[2], bg_color[3]),
        ])
        .build::<LinearGradient>()
        .unwrap();
    for (i, item) in cache.iter_mut().enumerate().take(256) {
        *item = gradient.at((i as f32) / 255.0).to_rgba8();
    }
}

fn cpu_pdf_dark_mode_shader(pixmap: &mut mupdf::Pixmap, gradient_cache: &[[u8; 4]; 256]) {
    let samples = pixmap.samples_mut();
    for pixel in samples.chunks_exact_mut(4) {
        let r: u16 = pixel[0] as u16;
        let g: u16 = pixel[1] as u16;
        let b: u16 = pixel[2] as u16;
        let brightness = ((r + g + b) / 3) as usize;
        let pixel_array: &mut [u8; 4] = pixel.try_into().unwrap();
        *pixel_array = gradient_cache[brightness];
    }
}

fn generate_key_combinations(count: usize) -> Vec<String> {
    // Use easily distinguishable characters (excluding confusing ones like 'I', 'l', 'O', '0')
    const CHARS: &[char] = &[
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'j', 'k', 'm', 'n', 'p', 'q', 'r', 's', 't', 'u',
        'v', 'w', 'x', 'y', 'z',
    ];

    let mut keys = Vec::new();

    for &c in CHARS.iter().take(count.min(CHARS.len())) {
        keys.push(c.to_string());
    }

    if count > CHARS.len() {
        let remaining = count - CHARS.len();
        let mut added = 0;
        'outer: for &c1 in CHARS {
            for &c2 in CHARS {
                if added >= remaining {
                    break 'outer;
                }
                keys.push(format!("{}{}", c1, c2));
                added += 1;
            }
        }
    }

    keys
}

fn get_background_color(invert_colors: bool) -> iced::Color {
    if invert_colors {
        iced::Color::from_rgb8(21, 22, 32)
    } else {
        iced::Color::from_rgb8(220, 219, 218)
    }
}
