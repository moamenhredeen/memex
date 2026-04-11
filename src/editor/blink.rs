use gpui::*;

pub struct BlinkCursor {
    visible: bool,
    epoch: usize,
    _task: Task<()>,
}

impl BlinkCursor {
    pub fn new() -> Self {
        Self {
            visible: true,
            epoch: 0,
            _task: Task::ready(()),
        }
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.epoch += 1;
        self.blink(self.epoch, cx);
    }

    fn blink(&mut self, epoch: usize, cx: &mut Context<Self>) {
        self._task = cx.spawn(async move |this, cx| {
            loop {
                Timer::after(std::time::Duration::from_millis(500)).await;
                if let Some(this) = this.upgrade() {
                    let should_continue = this
                        .update(cx, |this, cx| {
                            if this.epoch != epoch {
                                return false;
                            }
                            this.visible = !this.visible;
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if !should_continue {
                        break;
                    }
                } else {
                    break;
                }
            }
        });
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.epoch += 1;
        let epoch = self.epoch;
        self._task = cx.spawn(async move |this, cx| {
            Timer::after(std::time::Duration::from_millis(300)).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.epoch == epoch {
                        this.blink(epoch, cx);
                    }
                })
                .ok();
            }
        });
        cx.notify();
    }

    pub fn visible(&self) -> bool {
        self.visible
    }
}
