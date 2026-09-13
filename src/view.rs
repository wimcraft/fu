//! Pure viewport and geometry math: no terminal or file I/O.

/// Rows reserved at the bottom of the pane: a horizontal position bar plus a
/// plain-text status line (see `terminal::horizontal_bar`/`status_line`).
pub const STATUS_ROWS: u16 = 2;

/// Columns reserved on the right of the pane for the vertical position bar
/// (see `terminal::vertical_bar`). Neither is available to the image itself.
pub const STATUS_COLS: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub columns: u16,
    pub rows: u16,
    pub draw_columns: u16,
    pub draw_rows: u16,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

impl Geometry {
    pub fn new(columns: u16, rows: u16, pixel_width: u32, pixel_height: u32) -> Self {
        let draw_columns = columns.saturating_sub(STATUS_COLS).max(1);
        let draw_rows = rows.saturating_sub(STATUS_ROWS).max(1);
        Self {
            columns,
            rows,
            draw_columns,
            draw_rows,
            pixel_width,
            pixel_height,
        }
    }

    /// Pixel width of the drawable (non-status) area, derived from the real
    /// per-cell pixel width so a reserved status column isn't drawn into.
    pub fn draw_pixel_width(&self) -> u32 {
        if self.columns == 0 {
            return self.pixel_width.max(1);
        }
        let cell_width = self.pixel_width as u64 / self.columns as u64;
        ((cell_width * self.draw_columns as u64) as u32).max(1)
    }

    /// Pixel height of the drawable (non-status) area, derived from the
    /// real per-cell pixel height so aspect ratio matches the terminal.
    pub fn draw_pixel_height(&self) -> u32 {
        if self.rows == 0 {
            return self.pixel_height.max(1);
        }
        let cell_height = self.pixel_height as u64 / self.rows as u64;
        ((cell_height * self.draw_rows as u64) as u32).max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    Width,
    Whole,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub zoom: f32,
    pub x: u32,
    pub y: u32,
    pub fit: FitMode,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            x: 0,
            y: 0,
            fit: FitMode::Width,
        }
    }
}

/// Crop width/height for `zoom`, independent of the current pan position.
fn crop_dims(zoom: f32, geometry: &Geometry, source_w: u32, source_h: u32) -> (u32, u32) {
    let draw_w = geometry.draw_pixel_width() as f64;
    let draw_h = geometry.draw_pixel_height() as f64;
    let zoom = zoom.max(1.0) as f64;

    let crop_w = ((source_w as f64 / zoom).round() as u32).clamp(1, source_w.max(1));
    let crop_h = ((crop_w as f64 * draw_h / draw_w).round() as u32).clamp(1, source_h.max(1));
    (crop_w, crop_h)
}

/// Clamp a candidate crop origin so `[origin, origin + extent)` stays inside `[0, source)`.
fn clamp_extent(origin: u32, extent: u32, source: u32) -> u32 {
    origin.min(source.saturating_sub(extent))
}

/// Choose an origin that centres `extent` on `center`, then clamps to bounds.
fn center_origin(center: f64, extent: u32, source: u32) -> u32 {
    if extent >= source {
        return 0;
    }
    let max_origin = (source - extent) as f64;
    let origin = (center - extent as f64 / 2.0).clamp(0.0, max_origin);
    origin.round() as u32
}

impl View {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn crop(&self, geometry: &Geometry, source_w: u32, source_h: u32) -> Crop {
        let (width, height) = match self.fit {
            FitMode::Width => crop_dims(self.zoom, geometry, source_w, source_h),
            FitMode::Whole => (source_w.max(1), source_h.max(1)),
        };
        Crop {
            x: clamp_extent(self.x, width, source_w),
            y: clamp_extent(self.y, height, source_h),
            width,
            height,
        }
    }

    /// Re-clamp `x`/`y` in place against the current zoom/geometry/source.
    pub fn clamp(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        let c = self.crop(geometry, source_w, source_h);
        self.x = c.x;
        self.y = c.y;
    }

    fn center(&self, geometry: &Geometry, source_w: u32, source_h: u32) -> (f64, f64) {
        let c = self.crop(geometry, source_w, source_h);
        (
            c.x as f64 + c.width as f64 / 2.0,
            c.y as f64 + c.height as f64 / 2.0,
        )
    }

    fn set_zoom(&mut self, new_zoom: f32, geometry: &Geometry, source_w: u32, source_h: u32) {
        let (cx, cy) = self.center(geometry, source_w, source_h);
        self.zoom = new_zoom.max(1.0);
        self.fit = FitMode::Width;
        let (w, h) = crop_dims(self.zoom, geometry, source_w, source_h);
        self.x = center_origin(cx, w, source_w);
        self.y = center_origin(cy, h, source_h);
    }

    pub fn zoom_in(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        if self.fit == FitMode::Whole {
            self.fit = FitMode::Width;
            self.zoom = 1.0;
            self.x = 0;
            self.y = 0;
        } else {
            self.set_zoom(self.zoom + 1.0, geometry, source_w, source_h);
        }
    }

    pub fn zoom_out(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        if self.fit == FitMode::Whole {
            return;
        }
        if self.zoom <= 1.0 {
            self.fit_whole();
        } else {
            self.set_zoom(self.zoom - 1.0, geometry, source_w, source_h);
        }
    }

    pub fn fit_whole(&mut self) {
        self.fit = FitMode::Whole;
        self.zoom = 1.0;
        self.x = 0;
        self.y = 0;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn page_down(&mut self, geometry: &Geometry, source_w: u32, source_h: u32, overlap: f32) {
        let c = self.crop(geometry, source_w, source_h);
        let delta = (c.height as f32 * (1.0 - overlap)).round() as u32;
        self.y = clamp_extent(self.y.saturating_add(delta), c.height, source_h);
    }

    pub fn page_up(&mut self, geometry: &Geometry, source_w: u32, source_h: u32, overlap: f32) {
        let c = self.crop(geometry, source_w, source_h);
        let delta = (c.height as f32 * (1.0 - overlap)).round() as u32;
        self.y = self.y.saturating_sub(delta);
    }

    pub fn nudge_down(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        let c = self.crop(geometry, source_w, source_h);
        let delta = (c.height / 8).max(1);
        self.y = clamp_extent(self.y.saturating_add(delta), c.height, source_h);
    }

    pub fn nudge_up(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        let c = self.crop(geometry, source_w, source_h);
        let delta = (c.height / 8).max(1);
        self.y = self.y.saturating_sub(delta);
    }

    pub fn pan_left(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        let c = self.crop(geometry, source_w, source_h);
        let delta = (c.width / 2).max(1);
        self.x = self.x.saturating_sub(delta);
    }

    pub fn pan_right(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        let c = self.crop(geometry, source_w, source_h);
        let delta = (c.width / 2).max(1);
        self.x = clamp_extent(self.x.saturating_add(delta), c.width, source_w);
    }

    pub fn pan_left_full(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        let c = self.crop(geometry, source_w, source_h);
        self.x = self.x.saturating_sub(c.width.max(1));
    }

    pub fn pan_right_full(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        let c = self.crop(geometry, source_w, source_h);
        self.x = clamp_extent(self.x.saturating_add(c.width.max(1)), c.width, source_w);
    }

    pub fn top(&mut self) {
        self.y = 0;
    }

    pub fn bottom(&mut self, geometry: &Geometry, source_w: u32, source_h: u32) {
        let c = self.crop(geometry, source_w, source_h);
        self.y = source_h.saturating_sub(c.height);
    }

    /// Preserve the source-space viewport centre across a geometry change
    /// (terminal resize), then clamp to the new bounds.
    pub fn on_resize(
        &mut self,
        old_geometry: &Geometry,
        new_geometry: &Geometry,
        source_w: u32,
        source_h: u32,
    ) {
        let (cx, cy) = self.center(old_geometry, source_w, source_h);
        if self.fit == FitMode::Whole {
            self.x = 0;
            self.y = 0;
            return;
        }
        let (w, h) = crop_dims(self.zoom, new_geometry, source_w, source_h);
        self.x = center_origin(cx, w, source_w);
        self.y = center_origin(cy, h, source_h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(columns: u16, rows: u16, pixel_width: u32, pixel_height: u32) -> Geometry {
        Geometry::new(columns, rows, pixel_width, pixel_height)
    }

    #[test]
    fn initial_view_is_top_left() {
        let view = View::new();
        assert_eq!(view.zoom, 1.0);
        assert_eq!(view.x, 0);
        assert_eq!(view.y, 0);
        assert_eq!(view.fit, FitMode::Width);
    }

    #[test]
    fn initial_crop_fits_pane_width_at_top_left() {
        // 100 cols -> 99 draw cols (STATUS_COLS = 1); 41 rows -> 39 draw rows (STATUS_ROWS = 2)
        let g = geometry(100, 41, 1000, 800);
        let view = View::new();
        let c = view.crop(&g, 4000, 3000);
        assert_eq!(c.x, 0);
        assert_eq!(c.y, 0);
        // Crop width is always the full source width at zoom 1, regardless
        // of the reserved status column.
        assert_eq!(c.width, 4000);
        // draw_pixel_width = (1000/100)*99 = 990; draw_pixel_height = (800/41)*39 = 19*39 = 741
        // height = round(4000 * 741/990) = 2994
        assert_eq!(c.height, 2994);
    }

    #[test]
    fn crop_clamps_x_to_source_bounds() {
        let g = geometry(80, 40, 800, 600);
        let mut view = View::new();
        view.x = 10_000;
        view.clamp(&g, 2000, 1000);
        assert!(view.x <= 2000);
        let c = view.crop(&g, 2000, 1000);
        assert_eq!(c.x + c.width, c.x + c.width); // no overflow
        assert!(c.x + c.width <= 2000);
    }

    #[test]
    fn crop_clamps_y_to_source_bounds() {
        let g = geometry(80, 40, 800, 600);
        let mut view = View::new();
        view.y = 10_000;
        view.clamp(&g, 2000, 1000);
        let c = view.crop(&g, 2000, 1000);
        assert!(c.y + c.height <= 1000);
    }

    #[test]
    fn zoom_two_halves_crop_width() {
        let g = geometry(80, 40, 800, 600);
        let mut view = View::new();
        view.zoom_in(&g, 2000, 1000);
        assert_eq!(view.zoom, 2.0);
        let c = view.crop(&g, 2000, 1000);
        assert_eq!(c.width, 1000);
    }

    #[test]
    fn zoom_never_drops_below_one() {
        let g = geometry(80, 40, 800, 600);
        let mut view = View::new();
        view.zoom_out(&g, 2000, 1000);
        assert_eq!(view.zoom, 1.0);
    }

    #[test]
    fn zooming_out_from_width_fit_shows_the_whole_image() {
        let g = geometry(80, 40, 800, 600);
        let mut view = View::new();
        view.zoom_out(&g, 500, 2000);
        assert_eq!(view.fit, FitMode::Whole);
        assert_eq!(
            view.crop(&g, 500, 2000),
            Crop {
                x: 0,
                y: 0,
                width: 500,
                height: 2000,
            }
        );
    }

    #[test]
    fn zooming_in_from_whole_fit_returns_to_width_fit() {
        let g = geometry(80, 40, 800, 600);
        let mut view = View::new();
        view.fit_whole();
        view.zoom_in(&g, 500, 2000);
        assert_eq!(view.fit, FitMode::Width);
        assert_eq!(view.zoom, 1.0);
    }

    #[test]
    fn zoom_preserves_center_when_legal() {
        let g = geometry(80, 40, 800, 400); // square-ish draw area
        let mut view = View::new();
        view.x = 800;
        view.y = 400;
        view.clamp(&g, 2000, 1000);
        let (cx_before, cy_before) = view.center(&g, 2000, 1000);

        view.zoom_in(&g, 2000, 1000);
        let (cx_after, cy_after) = view.center(&g, 2000, 1000);

        assert!((cx_before - cx_after).abs() < 1.5);
        assert!((cy_before - cy_after).abs() < 1.5);
    }

    #[test]
    fn zoom_clamps_center_near_edges() {
        let g = geometry(80, 40, 800, 400);
        // A tightly zoomed-in crop pinned at the top-left edge has a centre
        // very close to (0, 0); zooming in further must clamp to zero
        // rather than underflow.
        let mut view = View {
            zoom: 500.0,
            x: 0,
            y: 0,
            fit: FitMode::Width,
        };
        view.zoom_in(&g, 2000, 1000);
        let c = view.crop(&g, 2000, 1000);
        assert_eq!(c.x, 0);
        assert_eq!(c.y, 0);
    }

    #[test]
    fn page_down_moves_by_height_minus_overlap() {
        let g = geometry(80, 40, 800, 400);
        let mut view = View::new();
        let before = view.crop(&g, 2000, 4000);
        view.page_down(&g, 2000, 4000, 0.12);
        let expected = (before.height as f32 * 0.88).round() as u32;
        assert_eq!(view.y, expected);
    }

    #[test]
    fn page_up_moves_back_by_same_amount() {
        let g = geometry(80, 40, 800, 400);
        let mut view = View::new();
        view.page_down(&g, 2000, 4000, 0.12);
        let after_down = view.y;
        view.page_up(&g, 2000, 4000, 0.12);
        assert!(view.y < after_down);
    }

    #[test]
    fn page_down_clamps_at_bottom() {
        let g = geometry(80, 40, 800, 400);
        let mut view = View::new();
        for _ in 0..100 {
            view.page_down(&g, 2000, 4000, 0.12);
        }
        let c = view.crop(&g, 2000, 4000);
        assert_eq!(c.y + c.height, 4000);
    }

    #[test]
    fn reset_restores_initial_view() {
        let g = geometry(80, 40, 800, 400);
        let mut view = View::new();
        view.zoom_in(&g, 2000, 1000);
        view.page_down(&g, 2000, 1000, 0.12);
        view.reset();
        assert_eq!(view.zoom, 1.0);
        assert_eq!(view.x, 0);
        assert_eq!(view.y, 0);
    }

    #[test]
    fn resize_preserves_source_center() {
        let old_g = geometry(80, 40, 800, 400);
        let new_g = geometry(120, 50, 1200, 500);
        let mut view = View::new();
        view.zoom_in(&old_g, 2000, 1000);
        view.x = 400;
        view.y = 200;
        view.clamp(&old_g, 2000, 1000);
        let (cx_before, cy_before) = view.center(&old_g, 2000, 1000);

        view.on_resize(&old_g, &new_g, 2000, 1000);

        let (cx_after, cy_after) = view.center(&new_g, 2000, 1000);
        assert!((cx_before - cx_after).abs() < 2.0);
        assert!((cy_before - cy_after).abs() < 2.0);
    }

    #[test]
    fn nudge_moves_by_eighth_of_height() {
        let g = geometry(80, 40, 800, 400);
        let mut view = View::new();
        let c = view.crop(&g, 2000, 4000);
        view.nudge_down(&g, 2000, 4000);
        assert_eq!(view.y, (c.height / 8).max(1));
    }

    #[test]
    fn pan_h_l_move_by_half_width() {
        let g = geometry(80, 40, 800, 400);
        let mut view = View::new();
        view.zoom_in(&g, 4000, 1000);
        view.x = 1000;
        view.clamp(&g, 4000, 1000);
        let c = view.crop(&g, 4000, 1000);
        let before = view.x;
        view.pan_left(&g, 4000, 1000);
        assert_eq!(view.x, before - (c.width / 2).max(1));
    }

    #[test]
    fn pan_capital_h_l_move_by_full_width() {
        let g = geometry(80, 40, 800, 400);
        let mut view = View::new();
        view.zoom_in(&g, 4000, 1000);
        view.x = 1000;
        view.clamp(&g, 4000, 1000);
        let c = view.crop(&g, 4000, 1000);
        let before = view.x;
        view.pan_right_full(&g, 4000, 1000);
        assert_eq!(view.x, clamp_extent(before + c.width, c.width, 4000));
    }

    #[test]
    fn g_and_shift_g_jump_to_top_and_bottom() {
        let g = geometry(80, 40, 800, 400);
        let mut view = View::new();
        view.y = 500;
        view.clamp(&g, 2000, 4000);
        view.top();
        assert_eq!(view.y, 0);
        view.bottom(&g, 2000, 4000);
        let c = view.crop(&g, 2000, 4000);
        assert_eq!(view.y + c.height, 4000);
    }
}
