use rusty_box_devices::composition::write_and_publish;

use super::{
    BxPhyAddress, Emit, IsaIrq, MmioModel, MmioWrite, PcAttachment, PcDma, PcInterrupt,
    PcMmioDevice, Reaction, SubscriberError, Subscriptions, WindowOffset,
};

/// Adapts a model and its local subscribers to the PC's scalar MMIO route.
pub struct EventEndpoint<D: MmioModel, S = Subscriptions<<D as MmioModel>::Event, SubscriberError>>
{
    model: D,
    subscribers: S,
}

impl<D: MmioModel + Send> PcAttachment<EventEndpoint<D>> {
    /// Prepare a placement for a model, with no subscribers yet.
    pub fn with_model(page: BxPhyAddress, line: IsaIrq, model: D) -> Self {
        Self::new(
            page,
            line,
            EventEndpoint {
                model,
                subscribers: Subscriptions::new(),
            },
        )
    }
}

impl<D, S> PcAttachment<EventEndpoint<D, S>>
where
    D: MmioModel + Send,
    S: for<'a> Emit<D::Event, PcInterrupt<'a>, Error = SubscriberError> + Send,
{
    /// Observe each notification produced by this model.
    pub fn on<F: FnMut(&D::Event) + Send>(
        self,
        handler: F,
    ) -> PcAttachment<
        EventEndpoint<
            D,
            impl for<'a> Emit<D::Event, PcInterrupt<'a>, Error = SubscriberError> + Send,
        >,
    > {
        self.with_subscribers(|previous| Subscriptions::new().forward(previous).on(handler))
    }

    /// Observe one typed selection of the model's notifications.
    pub fn on_event<E, F: FnMut(&E) + Send>(
        self,
        select: impl Fn(&D::Event) -> Option<&E> + Send,
        mut handler: F,
    ) -> PcAttachment<
        EventEndpoint<
            D,
            impl for<'a> Emit<D::Event, PcInterrupt<'a>, Error = SubscriberError> + Send,
        >,
    > {
        self.on(move |event| {
            if let Some(selected) = select(event) {
                handler(selected);
            }
        })
    }

    /// Observe notifications and optionally stop or report a host error.
    pub fn try_on<F: FnMut(&D::Event) -> Result<Reaction, SubscriberError> + Send>(
        self,
        handler: F,
    ) -> PcAttachment<
        EventEndpoint<
            D,
            impl for<'a> Emit<D::Event, PcInterrupt<'a>, Error = SubscriberError> + Send,
        >,
    > {
        self.with_subscribers(|previous| Subscriptions::new().forward(previous).try_on(handler))
    }

    /// Observe notifications with this endpoint's short IRQ loan.
    pub fn on_with_irq<F>(
        self,
        mut handler: F,
    ) -> PcAttachment<
        EventEndpoint<
            D,
            impl for<'a> Emit<D::Event, PcInterrupt<'a>, Error = SubscriberError> + Send,
        >,
    >
    where
        F: for<'a> FnMut(&D::Event, &mut PcInterrupt<'a>) -> Result<Reaction, SubscriberError>
            + Send,
    {
        self.with_subscribers(|previous| {
            Subscriptions::new()
                .forward(previous)
                .on_with(move |event: &D::Event, irq: &mut PcInterrupt<'_>| handler(event, irq))
        })
    }

    fn with_subscribers<R>(self, extend: impl FnOnce(S) -> R) -> PcAttachment<EventEndpoint<D, R>> {
        let (page, line, EventEndpoint { model, subscribers }) = self.into_parts();
        PcAttachment {
            page,
            line,
            device: EventEndpoint {
                model,
                subscribers: extend(subscribers),
            },
        }
    }
}

impl<D, S> PcMmioDevice for EventEndpoint<D, S>
where
    D: MmioModel + Send,
    S: for<'a> Emit<D::Event, PcInterrupt<'a>, Error = SubscriberError> + Send,
{
    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        self.model.read(offset, len)
    }

    fn write(
        &mut self,
        access: MmioWrite,
        dma: &mut PcDma<'_>,
        irq: &mut PcInterrupt<'_>,
    ) -> Result<Reaction, SubscriberError> {
        write_and_publish(&mut self.model, access, dma, irq, &mut self.subscribers)
    }
}
