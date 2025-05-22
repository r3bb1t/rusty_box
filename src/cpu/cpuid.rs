pub(crate) trait BxCpuIdTrait {
    const NAME: &'static str;

    fn init(&mut self) {}

    fn get_cpu_extensions(extensions: &[u32]) {}
}
