pub use openmat_plot_mir::{DataPoint, DataRect, InteractionError, ViewTransform2D};

#[cfg(test)]
mod tests {
    use openmat_plot_mir::{
        CssPoint, CssRect, DevicePixelRatio, DeviceRect, InteractionError, Viewport,
    };

    use super::*;

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn viewport(dpr: f64) -> Viewport {
        Viewport {
            css: CssRect::new(10.0, 20.0, 400.0, 200.0).unwrap(),
            device: DeviceRect {
                x: 20,
                y: 40,
                width: (400.0 * dpr).round() as u32,
                height: (200.0 * dpr).round() as u32,
            },
            device_pixel_ratio: DevicePixelRatio::new(dpr).unwrap(),
        }
    }

    fn transform(dpr: f64) -> ViewTransform2D {
        ViewTransform2D::new(
            viewport(dpr),
            DataRect::new(-10.0, 30.0, 100.0, 300.0).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn css_data_mapping_is_reversible_across_dpr_values() {
        for dpr in [1.0, 1.25, 2.0, 3.0] {
            let view = transform(dpr);
            let data = DataPoint::new(7.25, 234.5).unwrap();
            let css = view.data_to_css(data).unwrap();
            let round_trip = view.css_to_data(css).unwrap();
            assert!((round_trip.x - data.x).abs() < 2.0e-5);
            assert!((round_trip.y - data.y).abs() < 2.0e-5);
        }
    }

    #[test]
    fn zoom_preserves_pointer_anchor_and_pan_can_leave_home() {
        let mut view = transform(2.0);
        let anchor = CssPoint::new(310.0, 70.0).unwrap();
        let before = view.css_to_data(anchor).unwrap();
        view.zoom_css(anchor, 0.5).unwrap();
        let after = view.css_to_data(anchor).unwrap();
        assert!((before.x - after.x).abs() < 1.0e-5);
        assert!((before.y - after.y).abs() < 1.0e-5);

        view.home();
        let home = view.home_limits();
        view.pan_css(40.0, -20.0).unwrap();
        let limits = view.current_limits();
        assert_ne!(limits, home);
        assert!(limits.min.x < home.min.x);
        assert!(limits.min.y < home.min.y);

        let before = view.css_to_data(anchor).unwrap();
        view.zoom_css(anchor, 2.0).unwrap();
        let after = view.css_to_data(anchor).unwrap();
        assert!((before.x - after.x).abs() < 1.0e-5);
        assert!((before.y - after.y).abs() < 1.0e-5);
        assert!(view.current_limits().min.x < home.min.x);

        view.home();
        assert_eq!(view.current_limits(), home);
    }

    #[test]
    fn box_zoom_clamps_pointer_to_viewport_and_home_restores_exactly() {
        let mut view = transform(1.25);
        view.box_zoom_css(
            CssPoint::new(-100.0, 70.0).unwrap(),
            CssPoint::new(210.0, 500.0).unwrap(),
        )
        .unwrap();
        let limits = view.current_limits();
        assert!((limits.min.x + 10.0).abs() < f64::EPSILON);
        assert!((limits.min.y - 100.0).abs() < f64::EPSILON);
        assert!(limits.max.x < 30.0);
        assert!(limits.max.y < 300.0);
        view.home();
        assert_eq!(view.current_limits(), view.home_limits());
    }

    #[test]
    fn rejects_non_finite_zero_span_and_overflow() {
        assert_eq!(
            DataRect::new(0.0, 0.0, 0.0, 1.0),
            Err(InteractionError::ZeroSpan("data limits"))
        );
        assert!(matches!(
            DataRect::new(-f64::MAX, f64::MAX, 0.0, 1.0),
            Err(InteractionError::Overflow(_))
        ));
        let mut view = transform(1.0);
        assert_eq!(
            view.zoom_css(CssPoint::new(0.0, 0.0).unwrap(), f64::NAN),
            Err(InteractionError::NonFinite("zoom factor"))
        );
        assert_eq!(
            view.box_zoom_css(
                CssPoint::new(30.0, 30.0).unwrap(),
                CssPoint::new(30.0, 60.0).unwrap()
            ),
            Err(InteractionError::BoxTooSmall)
        );
    }
}
