use anyhow::Result;
use gpui::{Bounds, DisplayId, Pixels, PlatformDisplay, Point, Size, px};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct IosDisplayState {
    pub logical_width: f32,
    pub logical_height: f32,
    pub scale_factor: f32,
}

#[derive(Debug)]
pub struct IosDisplay {
    id: DisplayId,
    uuid: uuid::Uuid,
    state: Arc<Mutex<IosDisplayState>>,
}

impl IosDisplay {
    pub fn new(state: Arc<Mutex<IosDisplayState>>) -> Self {
        Self {
            id: DisplayId::new(1),
            uuid: uuid::Uuid::new_v4(),
            state,
        }
    }
}

impl PlatformDisplay for IosDisplay {
    fn id(&self) -> DisplayId {
        self.id
    }

    fn uuid(&self) -> Result<uuid::Uuid> {
        Ok(self.uuid)
    }

    fn bounds(&self) -> Bounds<Pixels> {
        let state = self.state.lock();
        Bounds {
            origin: Point::default(),
            size: Size {
                width: px(state.logical_width.max(1.0)),
                height: px(state.logical_height.max(1.0)),
            },
        }
    }
}
