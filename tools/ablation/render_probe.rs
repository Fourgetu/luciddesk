    #[test]
    fn ablation_render_measurements() {
        use std::{hint::black_box, sync::atomic::Ordering, time::Instant};
        use super::super::{label, native_graphics};
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let emit = |scenario: &str, mut samples: Vec<u128>, count: usize| {
            samples.sort_unstable();
            println!("ABLATION_METRIC {{\"scenario\":\"{scenario}\",\"samples\":{},\"p50_ns\":{},\"p95_ns\":{},\"work_count\":{count}}}", samples.len(), samples[(samples.len()-1)/2], samples[(samples.len()*95).div_ceil(100)-1]);
        };
        let device = native_graphics::gpu_device().unwrap();
        let before = native_graphics::ABLATION_CREATIONS.load(Ordering::Relaxed);
        let mut samples = Vec::new();
        for _ in 0..30 {
            let start = Instant::now();
            black_box(native_graphics::gpu_device().unwrap());
            samples.push(start.elapsed().as_nanos());
        }
        emit("device_acquire", samples, native_graphics::ABLATION_CREATIONS.load(Ordering::Relaxed)-before);
        for dpi in [96, 144, 192] {
            let scale = dpi as f32 / 96.0;
            let labels: Vec<_> = (0..40).map(|i| format!("消融实验文件 {i:02} 中文 ABC.txt")).collect();
            for text in &labels { label::layout_scaled(text, dpi, dpi, 2, 1.0).unwrap(); }
            let before = label::ABLATION_LAYOUTS.load(Ordering::Relaxed);
            let mut samples = Vec::new();
            for _ in 0..30 {
                let start = Instant::now();
                for text in &labels { black_box(label::layout_scaled(text, dpi, dpi, 2, 1.0).unwrap()); }
                samples.push(start.elapsed().as_nanos());
            }
            emit(&format!("layout_40_dpi_{dpi}"), samples, label::ABLATION_LAYOUTS.load(Ordering::Relaxed)-before);
            let mut model = sample_model();
            let original = model.items[0].clone();
            model.items = (0..24).map(|i| {
                let mut item = original.clone();
                item.label = labels[i].clone();
                item.identity = desktop_core::ShellIdentity::Namespace { parsing_name: format!("ablation:{i}") };
                // Distinct source identities exercise per-icon caching.
                item.image = original.image.as_ref().map(|image| Arc::new((**image).clone()));
                item
            }).collect();
            let (width, height) = ((640.0*scale) as u32, (480.0*scale) as u32);
            let bitmap = canvas::Offscreen::new(&device, width, height).unwrap();
            let mut renderer = Renderer::new().unwrap();
            renderer.paint(&bitmap.target, width, height, scale, &model).unwrap();
            let expected = bitmap.pixels().unwrap();
            assert!(expected.chunks_exact(4).any(|p| p[3] != 0));
            let before = super::ABLATION_UPLOADS.load(Ordering::Relaxed);
            let mut draw = Vec::new();
            let mut complete = Vec::new();
            for _ in 0..30 {
                let start = Instant::now();
                renderer.paint(&bitmap.target, width, height, scale, &model).unwrap();
                draw.push(start.elapsed().as_nanos());
                let pixels = bitmap.pixels().unwrap();
                complete.push(start.elapsed().as_nanos());
                assert_eq!(pixels, expected, "warm redraw changed pixels at DPI {dpi}");
            }
            let uploads = super::ABLATION_UPLOADS.load(Ordering::Relaxed)-before;
            emit(&format!("paint_dpi_{dpi}"), draw, uploads);
            emit(&format!("paint_readback_dpi_{dpi}"), complete, uploads);
            let output = std::path::PathBuf::from(std::env::var_os("LUCIDPANE_ABLATION_OUTPUT").unwrap());
            std::fs::write(output.join(format!("pane-{dpi}.bgra")), expected).unwrap();
        }
    }
