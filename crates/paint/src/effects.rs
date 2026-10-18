use crate::Canvas;

pub(super) struct Layer {
    pixels: Option<Vec<u8>>,
    opacity: f32,
    suppressed: bool,
}
impl Canvas {
    pub(super) fn push_opacity(&mut self, opacity: f32) {
        let retained: usize = self
            .layers
            .iter()
            .filter_map(|l| l.pixels.as_ref())
            .map(Vec::len)
            .sum();
        let mut pixels = Vec::new();
        let available = retained <= (128 * 1024 * 1024usize).saturating_sub(self.px.len());
        let allocated =
            available && !self.suppress && pixels.try_reserve_exact(self.px.len()).is_ok();
        let suppressed = self.suppress;
        let pixels = if allocated {
            pixels.resize(self.px.len(), 0);
            Some(std::mem::replace(&mut self.px, pixels))
        } else {
            self.suppress = true;
            None
        };
        self.layers.push(Layer {
            pixels,
            opacity: opacity.clamp(0.0, 1.0),
            suppressed,
        });
    }
    pub(super) fn pop_opacity(&mut self) {
        let Some(layer) = self.layers.pop() else {
            return;
        };
        self.suppress = layer.suppressed;
        if let Some(mut parent) = layer.pixels {
            // The group's pixels already contain clip coverage. Compositing via
            // blend() would apply rounded edge masks a second time and darken them.
            for (source, destination) in self
                .px
                .as_chunks::<4>()
                .0
                .iter()
                .zip(parent.as_chunks_mut::<4>().0.iter_mut())
            {
                let alpha = source[3] as f32 / 255.0 * layer.opacity;
                let remaining = destination[3] as f32 / 255.0 * (1.0 - alpha);
                let total = alpha + remaining;
                if total == 0.0 {
                    destination.fill(0);
                    continue;
                }
                for channel in 0..3 {
                    destination[channel] = ((source[channel] as f32 * alpha
                        + destination[channel] as f32 * remaining)
                        / total)
                        .round() as u8;
                }
                destination[3] = (total * 255.0).round() as u8;
            }
            self.px = parent;
        }
    }
}
