pub(crate) trait BxCpuTrait {
    const NAME: &'static str;

    fn init(&mut self) {}

    fn get_cpu_extensions(extensions: &[u32]) {}
}
