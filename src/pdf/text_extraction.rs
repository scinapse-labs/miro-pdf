#[cfg(test)]
mod tests {
    use anyhow::Result;
    use iced::Size;

    use crate::geometry::{Rect, Vector};
    use crate::pdf::widget::PdfViewer;

    fn test_viewer() -> Result<PdfViewer> {
        let mut viewer = PdfViewer::from_path("assets/text-copy-test.pdf".into())?;
        viewer.set_viewport_for_test(Size::new(800.0, 600.0));
        Ok(viewer)
    }

    #[test]
    fn test_text_extraction_basic() -> Result<()> {
        let viewer = test_viewer()?;

        // Cover the entire viewport to capture all visible text.
        let screen_rect = Rect::from_pos_size(
            Vector::new(0.0, 0.0),
            Vector::new(800.0, 600.0),
        );

        let text = viewer.extract_text_from_rect(screen_rect);
        assert!(!text.is_empty());
        assert!(text.contains("Energy harvesting"));
        assert!(text.contains("Vincent Udén"));

        Ok(())
    }

    #[test]
    fn test_text_extraction_rectangle_selection() -> Result<()> {
        let viewer = test_viewer()?;

        // With the default viewport (800x600) and an A4 page (595x842),
        // the page is centered at:
        //   x0 = (800 - 595) / 2 = 102.5
        //   y0 = (600 - 842) / 2 = -121
        // The title "Energy harvesting" sits at roughly PDF (200, 299)-(394, 327).
        // In screen space that is:
        //   x0 = 200 + 102.5 = 302.5
        //   y0 = 299 + (-121) = 178
        //   x1 = 394 + 102.5 = 496.5
        //   y1 = 327 + (-121) = 206
        let selection_rect = Rect::from_points(
            Vector::new(300.0, 170.0),
            Vector::new(500.0, 210.0),
        );

        let text = viewer.extract_text_from_rect(selection_rect);
        assert!(!text.is_empty());
        assert!(text.contains("Energy harvesting"));

        Ok(())
    }

    #[test]
    fn test_multiple_pages() -> Result<()> {
        let viewer = test_viewer()?;

        let page_count = viewer.page_count()? as usize;
        assert!(page_count > 1);

        // Page 1 is below the viewport, so we need a taller rect to reach it.
        let screen_rect = Rect::from_pos_size(
            Vector::new(0.0, 0.0),
            Vector::new(800.0, 2000.0),
        );

        let text = viewer.extract_text_from_rect(screen_rect);
        assert!(text.contains("Introduction"));

        Ok(())
    }

    #[test]
    fn test_text_extraction_integration() -> Result<()> {
        let viewer = test_viewer()?;

        // Large rect covering both pages.
        let screen_rect = Rect::from_pos_size(
            Vector::new(0.0, 0.0),
            Vector::new(800.0, 2000.0),
        );

        let all_text = viewer.extract_text_from_rect(screen_rect);
        assert!(all_text.contains("Energy harvesting"));
        assert!(all_text.contains("Introduction"));

        // Select just the title area on page 0.
        let title_rect = Rect::from_points(
            Vector::new(300.0, 170.0),
            Vector::new(500.0, 210.0),
        );
        let title_text = viewer.extract_text_from_rect(title_rect);
        assert!(title_text.contains("Energy harvesting"));
        println!("Title selection: '{}'", title_text);

        // Select a region on page 1 (shifted down by one page height + gap).
        // With SinglePage layout, page 1 is roughly at y ≈ 842 + 10 = 852 below page 0's origin.
        // In screen space page 0 is at y0 = -121, so page 1 is at y0 ≈ -121 + 852 = 731.
        let intro_rect = Rect::from_points(
            Vector::new(100.0, 750.0),
            Vector::new(700.0, 900.0),
        );
        let intro_text = viewer.extract_text_from_rect(intro_rect);
        println!("Intro selection: '{}'", intro_text);

        Ok(())
    }

    #[test]
    fn test_screen_to_document_coordinate_simulation() -> Result<()> {
        let viewer = test_viewer()?;

        println!("=== COORDINATE CONVERSION SIMULATION ===");

        let screen_positions = vec![
            (400.0, 300.0),
            (200.0, 200.0),
            (600.0, 400.0),
        ];

        let viewport_bounds = Rect::from_pos_size(
            Vector::new(0.0, 0.0),
            Vector::new(800.0, 600.0),
        );

        let page_size = Vector::new(595.0, 842.0);
        let translation = Vector::new(0.0, 0.0);
        let scale = 1.0;

        println!("Viewport bounds: {viewport_bounds:?}");
        println!("Page size: {page_size:?}");
        println!("Translation: {translation:?}");
        println!("Scale: {scale}");

        let pdf_offset = Vector::new(
            -(viewport_bounds.width() - page_size.x * scale) / 2.0,
            -(viewport_bounds.height() - page_size.y * scale) / 2.0,
        );

        println!("PDF offset: {pdf_offset:?}");

        for (screen_x, screen_y) in screen_positions {
            let screen_pos = Vector::new(screen_x, screen_y);
            let viewport_relative = screen_pos - viewport_bounds.x0;
            let pdf_relative = viewport_relative - pdf_offset;
            let doc_pos = pdf_relative.scaled(1.0 / scale) + translation;

            println!(
                "Screen ({}, {}) -> Viewport rel ({}, {}) -> PDF rel ({}, {}) -> Doc ({}, {})",
                screen_x,
                screen_y,
                viewport_relative.x,
                viewport_relative.y,
                pdf_relative.x,
                pdf_relative.y,
                doc_pos.x,
                doc_pos.y
            );
        }

        println!("\n=== INTERSECTION TESTS ===");
        println!("Known text positions from coordinate debugging:");
        println!("- 'Energy harvesting': (200-394, 299-327)");
        println!("- 'Vincent Udén': (262-333, 362-376)");

        // Verify the viewer can extract text at those document coordinates.
        let title_text = viewer.extract_text_from_rect(Rect::from_points(
            Vector::new(300.0, 170.0),
            Vector::new(500.0, 210.0),
        ));
        assert!(title_text.contains("Energy harvesting"));

        Ok(())
    }
}
