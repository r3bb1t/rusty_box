use rusty_box::emulator::hardware::{
    Emit, MmioWrite, PcDma, PcInterrupt, PcMmioDevice, Reaction, WindowOffset,
};

use super::gpu::{self, Gpu, TileRejected, TileWritten};

/// The board adapter holds the model separately from its two event routes.
pub struct GpuEndpoint<W, R> {
    pub gpu: Gpu,
    pub written: W,
    pub rejected: R,
}

impl<W, R, X> PcMmioDevice for GpuEndpoint<W, R>
where
    W: for<'a> Emit<TileWritten, PcInterrupt<'a>, Error = X> + Send,
    R: for<'a> Emit<TileRejected, PcInterrupt<'a>, Error = X> + Send,
    X: core::fmt::Debug,
{
    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        self.gpu.read(offset, len)
    }

    fn write(
        &mut self,
        access: MmioWrite,
        dma: &mut PcDma<'_>,
        irq: &mut PcInterrupt<'_>,
    ) -> Reaction {
        match gpu::write(
            &mut self.gpu,
            access,
            dma,
            irq,
            &mut self.written,
            &mut self.rejected,
        ) {
            Ok(reaction) => reaction,
            Err(error) => {
                eprintln!("GPU subscriber failed: {error:?}");
                self.gpu.subscriber_failed();
                Reaction::Stop
            }
        }
    }
}

use core::convert::Infallible;

use rusty_box::config::BxPhyAddress;
use rusty_box::emulator::hardware::{IsaIrq, PcAttachment, Subscriptions};

/// The GPU's typed attachment. Each handler is registered on this instance.
/// The device's public event types live beside its implementation.
pub struct GpuAttachment<W = Subscriptions<TileWritten>, R = Subscriptions<TileRejected>> {
    endpoint: GpuEndpoint<W, R>,
}

impl GpuAttachment {
    pub fn new() -> Self {
        Self {
            endpoint: GpuEndpoint {
                gpu: Gpu::default(),
                written: Subscriptions::new(),
                rejected: Subscriptions::new(),
            },
        }
    }
}

impl<W, R> GpuAttachment<W, R>
where
    W: for<'a> Emit<TileWritten, PcInterrupt<'a>, Error = Infallible> + Send,
    R: for<'a> Emit<TileRejected, PcInterrupt<'a>, Error = Infallible> + Send,
{
    pub fn on(
        self,
        observer: impl FnMut(&TileWritten) + Send,
    ) -> GpuAttachment<impl for<'a> Emit<TileWritten, PcInterrupt<'a>, Error = Infallible> + Send, R>
    {
        let GpuEndpoint {
            gpu,
            written,
            rejected,
        } = self.endpoint;
        GpuAttachment {
            endpoint: GpuEndpoint {
                gpu,
                written: Subscriptions::<TileWritten>::new()
                    .forward(written)
                    .on(observer),
                rejected,
            },
        }
    }

    pub fn on_with_irq(
        self,
        mut handler: impl for<'a> FnMut(&TileWritten, &mut PcInterrupt<'a>) -> Reaction + Send,
    ) -> GpuAttachment<impl for<'a> Emit<TileWritten, PcInterrupt<'a>, Error = Infallible> + Send, R>
    {
        let GpuEndpoint {
            gpu,
            written,
            rejected,
        } = self.endpoint;
        GpuAttachment {
            endpoint: GpuEndpoint {
                gpu,
                written: Subscriptions::<TileWritten>::new()
                    .forward(written)
                    .on_with(move |event: &TileWritten, irq: &mut PcInterrupt<'_>| {
                        Ok(handler(event, irq))
                    }),
                rejected,
            },
        }
    }

    pub fn on_rejected(
        self,
        observer: impl FnMut(&TileRejected) + Send,
    ) -> GpuAttachment<W, impl for<'a> Emit<TileRejected, PcInterrupt<'a>, Error = Infallible> + Send>
    {
        let GpuEndpoint {
            gpu,
            written,
            rejected,
        } = self.endpoint;
        GpuAttachment {
            endpoint: GpuEndpoint {
                gpu,
                written,
                rejected: Subscriptions::<TileRejected>::new()
                    .forward(rejected)
                    .on(observer),
            },
        }
    }

    pub fn at(self, page: BxPhyAddress, line: IsaIrq) -> PcAttachment<impl PcMmioDevice> {
        PcAttachment::new(page, line, self.endpoint)
    }
}
