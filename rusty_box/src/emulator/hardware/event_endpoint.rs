use rusty_box_devices::composition::{MmioEvent, write_and_publish};

use super::{
    BxPhyAddress, Emit, ExecutionRequest, IsaIrq, MmioModel, MmioWrite, PcAttachment, PcDma,
    PcInterrupt, PcMmioDevice, SubscriberError, Subscriptions, WindowOffset,
};

/// Adapts a model and its local subscribers to the PC's scalar MMIO route.
pub struct EventEndpoint<D: MmioModel, S = Subscriptions<MmioEvent<D>, SubscriberError>> {
    model: D,
    subscribers: S,
}

impl<D: MmioModel + Send> EventEndpoint<D> {
    pub(super) fn new(model: D) -> Self {
        Self::with_subscriptions(model, Subscriptions::new())
    }
}

impl<D: MmioModel + Send, S> EventEndpoint<D, S>
where
    S: for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
{
    /// Pair a model with one instance's subscribers, before board placement.
    pub fn with_subscriptions(model: D, subscribers: S) -> Self {
        Self { model, subscribers }
    }

    pub(super) fn on<F: FnMut(&MmioEvent<D>) + Send>(
        self,
        handler: F,
    ) -> EventEndpoint<
        D,
        impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
    > {
        EventEndpoint::with_subscriptions(
            self.model,
            Subscriptions::new().forward(self.subscribers).on(handler),
        )
    }

    pub(super) fn try_on<
        F: FnMut(&MmioEvent<D>) -> Result<ExecutionRequest, SubscriberError> + Send,
    >(
        self,
        handler: F,
    ) -> EventEndpoint<
        D,
        impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
    > {
        EventEndpoint::with_subscriptions(
            self.model,
            Subscriptions::new()
                .forward(self.subscribers)
                .try_on(handler),
        )
    }

    pub(super) fn on_with_irq<F>(
        self,
        mut handler: F,
    ) -> EventEndpoint<
        D,
        impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
    >
    where
        F: for<'a> FnMut(
                &MmioEvent<D>,
                &mut PcInterrupt<'a>,
            ) -> Result<ExecutionRequest, SubscriberError>
            + Send,
    {
        EventEndpoint::with_subscriptions(
            self.model,
            Subscriptions::new().forward(self.subscribers).on_with(
                move |event: &MmioEvent<D>, irq: &mut PcInterrupt<'_>| handler(event, irq),
            ),
        )
    }
}

impl<D: MmioModel + Send> PcAttachment<EventEndpoint<D>> {
    /// Prepare a placement for a model, with no subscribers yet.
    pub fn with_model(page: BxPhyAddress, line: IsaIrq, model: D) -> Self {
        Self::new(page, line, EventEndpoint::new(model))
    }
}

impl<D, S> PcAttachment<EventEndpoint<D, S>>
where
    D: MmioModel + Send,
    S: for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
{
    /// Observe each notification produced by this model.
    pub fn on<F: FnMut(&MmioEvent<D>) + Send>(
        self,
        handler: F,
    ) -> PcAttachment<
        EventEndpoint<
            D,
            impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
        >,
    > {
        self.map_device(|device| device.on(handler))
    }

    /// Observe one typed selection of the model's notifications.
    pub fn on_event<E, F: FnMut(&E) + Send>(
        self,
        select: impl Fn(&MmioEvent<D>) -> Option<&E> + Send,
        mut handler: F,
    ) -> PcAttachment<
        EventEndpoint<
            D,
            impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
        >,
    > {
        self.on(move |event| {
            if let Some(selected) = select(event) {
                handler(selected);
            }
        })
    }

    /// Observe notifications and optionally stop or report a host error.
    pub fn try_on<F: FnMut(&MmioEvent<D>) -> Result<ExecutionRequest, SubscriberError> + Send>(
        self,
        handler: F,
    ) -> PcAttachment<
        EventEndpoint<
            D,
            impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
        >,
    > {
        self.map_device(|device| device.try_on(handler))
    }

    /// Observe notifications with this endpoint's short IRQ loan.
    pub fn on_with_irq<F>(
        self,
        handler: F,
    ) -> PcAttachment<
        EventEndpoint<
            D,
            impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
        >,
    >
    where
        F: for<'a> FnMut(
                &MmioEvent<D>,
                &mut PcInterrupt<'a>,
            ) -> Result<ExecutionRequest, SubscriberError>
            + Send,
    {
        self.map_device(|device| device.on_with_irq(handler))
    }

    fn map_device<R: PcMmioDevice>(
        self,
        map: impl FnOnce(EventEndpoint<D, S>) -> R,
    ) -> PcAttachment<R> {
        let (page, line, device) = self.into_parts();
        PcAttachment::new(page, line, map(device))
    }
}

impl<D, S> PcMmioDevice for EventEndpoint<D, S>
where
    D: MmioModel + Send,
    S: for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
{
    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        self.model.read(offset, len)
    }

    fn write(
        &mut self,
        access: MmioWrite,
        dma: &mut PcDma<'_>,
        irq: &mut PcInterrupt<'_>,
    ) -> Result<ExecutionRequest, SubscriberError> {
        write_and_publish(&mut self.model, access, dma, irq, &mut self.subscribers)
    }
}
