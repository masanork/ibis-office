use anyhow::Result;
use gpui::{Bounds, DisplayId, Pixels, PlatformDisplay, Point, Size, px};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AndroidDisplayState {
    pub physical_width: u32,
    pub physical_height: u32,
    pub scale_factor: f32,
}

#[derive(Debug)]
pub struct AndroidDisplay {
    id: DisplayId,
    uuid: uuid::Uuid,
    state: Arc<Mutex<AndroidDisplayState>>,
}

impl AndroidDisplay {
    pub fn new(state: Arc<Mutex<AndroidDisplayState>>) -> Self {
        Self {
            id: DisplayId::new(1),
            uuid: uuid::Uuid::new_v4(),
            state,
        }
    }
}

impl PlatformDisplay for AndroidDisplay {
    fn id(&self) -> DisplayId {
        self.id
    }

    fn uuid(&self) -> Result<uuid::Uuid> {
        Ok(self.uuid)
    }

    fn bounds(&self) -> Bounds<Pixels> {
        let state = self.state.lock();
        let scale = state.scale_factor.max(1.0);
        Bounds {
            origin: Point::default(),
            size: Size {
                width: px(state.physical_width as f32 / scale),
                height: px(state.physical_height as f32 / scale),
            },
        }
    }
}
