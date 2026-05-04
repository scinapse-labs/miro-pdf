use anyhow::{anyhow, Result};
use iced::Size;
use mupdf::{Document, Page, page};
use num::Integer;
use serde::{Deserialize, Serialize};
use strum::EnumString;

use crate::geometry::{Rect, Vector};

#[derive(Debug, Clone, Serialize, Deserialize, EnumString, Default)]
pub enum PageLayout {
    #[default]
    /// One page per row, many rows
    SinglePage,
    /// Two pages per row, many rows
    TwoPage,
    /// Two pages per row, many rows, except for the first page which is on its own
    TwoPageTitlePage,
    /// Only one page on the screen at a time
    Presentation,
}

impl PageLayout {
    const GAP: f32 = 10.0;

    /// Returns visible pages and their bounding boxes relative to the widgets origin. A translation
    /// of (0,0) should result in the first page row being centered on the screen. Scale is applied
    /// after translation with respect to the center of the screen. Thus zooming doesn't move the
    /// doucment.
    pub fn pages_rects(
        &self,
        doc: &Document,
        translation: Vector<f32>, // In document space
        scale: f32,
        fractional_scale: f32,
        viewport: Size<f32>,
    ) -> Result<Vec<Rect<f32>>> {
        let mut out = vec![];
        let mut pages = doc.pages()?;
        let vsize: Vector<_> = viewport.into();
        let effective_scale = scale * fractional_scale;
        match self {
            PageLayout::SinglePage => {
                let mut pos: Vector<f32> = Vector::zero();
                let mut prev_bounds = Rect::default();
                for (i, page) in pages.flatten().enumerate() {
                    let mut bounds: Rect<f32> = page.bounds()?.into();
                    bounds.translate((vsize - bounds.size()).scaled(0.5));
                    bounds.translate(translation.scaled(effective_scale));
                    bounds = bounds.scaled(effective_scale);
                    if i != 0 {
                        pos.y += (prev_bounds.height() + bounds.height()) / 2.0;
                    }
                    bounds.translate(pos);

                    pos.y += Self::GAP * effective_scale;
                    prev_bounds = bounds;

                    out.push(bounds.into());
                }
            }
            PageLayout::TwoPage => {
                let mut pos: Vector<f32> = Vector::zero();
                for (i, page) in pages.flatten().enumerate() {
                    let mut bounds: Rect<f32> = page.bounds()?.into();
                    bounds.translate(pos);
                    bounds.translate((vsize - bounds.size()).scaled(0.5));
                    bounds.translate(translation.scaled(effective_scale));
                    bounds = bounds.scaled(effective_scale);

                    if i.is_odd() {
                        pos.y += bounds.size().y;
                        pos.y += Self::GAP * effective_scale;
                        pos.x = 0.0;
                    } else {
                        pos.x += bounds.size().x;
                        pos.x += Self::GAP * effective_scale;
                    }

                    out.push(bounds.into());
                }
            }
            PageLayout::TwoPageTitlePage => {
                let mut pos: Vector<f32> = Vector::zero();
                let Some(Ok(first_page)) = pages.next() else {
                    return Ok(out);
                };
                let mut bounds: Rect<f32> = first_page.bounds()?.into();
                bounds.translate((vsize - bounds.size()).scaled(0.5));
                bounds.translate(translation.scaled(effective_scale));
                bounds = bounds.scaled(effective_scale);
                out.push(bounds.into());
                pos.y += Self::GAP * effective_scale + bounds.size().y;

                for (i, page) in pages.flatten().enumerate() {
                    let mut bounds: Rect<f32> = page.bounds()?.into();
                    bounds.translate(pos);
                    bounds.translate((vsize - bounds.size()).scaled(0.5));
                    bounds.translate(translation.scaled(effective_scale));
                    bounds = bounds.scaled(effective_scale);

                    if i.is_odd() {
                        pos.y += bounds.size().y;
                        pos.y += Self::GAP * effective_scale;
                        pos.x = 0.0;
                    } else {
                        pos.x += bounds.size().x;
                        pos.x += Self::GAP * effective_scale;
                    }

                    out.push(bounds.into());
                }

                if out.len() >= 3 {
                    let pages_below_width =
                        out[1].width() + out[2].width() + Self::GAP * effective_scale * 2.0;
                    out[0].translate(Vector::new(pages_below_width / 4.0, 0.0));

                    for bound in &mut out {
                        bound.translate(Vector::new(-pages_below_width / 4.0, 0.0));
                    }
                } else if out.len() == 2 {
                    let half_width = first_page.bounds()?.width() / 2.0;
                    out[1].translate(Vector::new(half_width, 0.0));
                }
            }
            PageLayout::Presentation => {
                let mut pos: Vector<f32> = Vector::zero();
                let mut prev_bounds = Rect::default();
                for (i, page) in pages.flatten().enumerate() {
                    let mut bounds: Rect<f32> = page.bounds()?.into();
                    bounds.translate((vsize - bounds.size()).scaled(0.5));
                    bounds.translate(translation.scaled(effective_scale));
                    bounds = bounds.scaled(effective_scale);
                    if i != 0 {
                        pos.y += (prev_bounds.height() + bounds.height()) / 2.0;
                    }
                    bounds.translate(pos);

                    pos.y += Self::GAP * effective_scale;
                    prev_bounds = bounds;

                    out.push(bounds.into());
                }

                if !out.is_empty() {
                    let viewport_center: Vector<f32> = vsize.scaled(0.5).into();
                    let closest = out
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| {
                            (a.center() - viewport_center)
                                .norm_squared()
                                .partial_cmp(&(b.center() - viewport_center).norm_squared())
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .map(|(i, _)| i)
                        .unwrap();

                    let snap_offset = viewport_center - out[closest].center();
                    for rect in &mut out {
                        rect.translate(snap_offset);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Returns the translation that would leave the page at [page_idx] visible on the screen. If
    /// `page_idx > doc.page_count()` this will move to the last page.
    pub fn translation_for_page(
        &self,
        doc: &Document,
        scale: f32,
        fractional_scale: f32,
        page_idx: usize,
        viewport: Size<f32>,
    ) -> Result<Vector<f32>> {
        let rects = self.pages_rects(doc, Vector::zero(), scale, fractional_scale, viewport)?;
        rects
            .get(page_idx)
            .map(|rect| rect.center())
            .ok_or(anyhow!("Page index {page_idx} out of bounds"))
    }

    pub fn current_page_index(
        &self,
        doc: &Document,
        translation: Vector<f32>,
        viewport: Size<f32>,
    ) -> Result<usize> {
        let rects = self.pages_rects(doc, -translation, 1.0, 1.0, viewport)?;
        let mut closest = 0;
        let viewport: Vector<_> = viewport.into();
        if rects.is_empty() {
            return Err(anyhow!("There are no pages"));
        }
        for (i, rect) in rects.iter().enumerate() {
            if (rect.center() - viewport.scaled(0.5)).norm_squared()
                < (rects[closest].center() - viewport.scaled(0.5)).norm_squared()
            {
                closest = i;
            }
        }
        return Ok(closest);
    }

    pub fn center_of_page(
        &self,
        doc: &Document,
        translation: Vector<f32>,
        viewport: Size<f32>,
    ) -> Result<Rect<f32>> {
        let rects = self.pages_rects(doc, translation, 1.0, 1.0, viewport)?;
        let mut idx = self.current_page_index(doc, translation, viewport)?;
        Ok(rects[idx])
    }

    pub fn center_of_page_above(
        &self,
        doc: &Document,
        translation: Vector<f32>,
        viewport: Size<f32>,
    ) -> Result<Rect<f32>> {
        let rects = self.pages_rects(doc, translation, 1.0, 1.0, viewport)?;
        let mut idx = self.current_page_index(doc, translation, viewport)?;
        idx = (match self {
            PageLayout::SinglePage => idx.saturating_sub(1),
            PageLayout::TwoPage => idx.saturating_sub(2),
            PageLayout::TwoPageTitlePage => idx.saturating_sub(2),
            PageLayout::Presentation => idx.saturating_sub(1),
        })
        .clamp(0, rects.len() - 1);
        Ok(rects[idx])
    }

    pub fn center_of_page_below(
        &self,
        doc: &Document,
        translation: Vector<f32>,
        viewport: Size<f32>,
    ) -> Result<Rect<f32>> {
        let rects = self.pages_rects(doc, translation, 1.0, 1.0, viewport)?;
        let mut idx = self.current_page_index(doc, translation, viewport)?;
        idx = (match self {
            PageLayout::SinglePage => idx + 1,
            PageLayout::TwoPage => idx + 2,
            PageLayout::TwoPageTitlePage => {
                if idx == 0 {
                    idx + 1
                } else {
                    idx + 2
                }
            }
            PageLayout::Presentation => idx + 1,
        })
        .clamp(0, rects.len() - 1);
        Ok(rects[idx])
    }
}
