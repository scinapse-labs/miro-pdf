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
        let pages = doc.pages()?;
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
            PageLayout::TwoPageTitlePage => todo!(),
            PageLayout::Presentation => todo!(),
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
        scale: f32,
        fractional_scale: f32,
        viewport: Size<f32>,
    ) -> Result<usize> {
        let rects = self.pages_rects(doc, translation, scale, fractional_scale, viewport)?;
        let mut closest = 0;
        if rects.is_empty() {
            return Err(anyhow!("There are no pages"));
        }
        for (i, rect) in rects.iter().enumerate() {
            if rect.center().norm_squared() < rects[closest].center().norm_squared() {
                closest = i;
            }
        }
        return Ok(closest);
    }

    /// Returns the height of the row of pages occupying the middle of the creen
    pub fn page_set_height(
        &self,
        doc: &Document,
        translation: Vector<f32>,
        scale: f32,
        fractional_scale: f32,
        viewport: Size<f32>,
    ) -> Result<f32> {
        let rects = self.pages_rects(doc, translation, scale, fractional_scale, viewport)?;
        match self {
            PageLayout::SinglePage => Ok(rects
                [self.current_page_index(doc, translation, scale, fractional_scale, viewport)?]
            .height()),
            PageLayout::TwoPage => {
                let idx = (self.current_page_index(
                    doc,
                    translation,
                    scale,
                    fractional_scale,
                    viewport,
                )? % 2
                    * 2)
                .min(doc.page_count()? as usize);

                let heights = vec![
                    rects.get(idx).map(|rect| rect.height()),
                    rects.get(idx + 1).map(|rect| rect.height()),
                ];

                Ok(heights.into_iter().flatten().fold(0.0, |a, b| a.max(b)))
            }
            PageLayout::TwoPageTitlePage => todo!(),
            PageLayout::Presentation => todo!(),
        }
    }
}
