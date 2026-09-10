#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::comparison_chain,
    clippy::float_cmp
)]
//! GeForce GPU Emulation Module
//!
//! Implements NVIDIA GeForce 2/3/FX 5900/6800 GPU emulation.
//! Ported from Bochs geforce.cc/geforce.h.

use rusty_box_core::FloatExt;
use alloc::vec;
use alloc::{boxed::Box, vec::Vec};

use crate::api::WindowOffset;
use crate::display::card::{MemCtx, PortCtx, ResetCtx, VgaExtension, Written};
use crate::display::ddc::BxDdcC;
use crate::display::vga::VgaWindow;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The last CRTC index the standard VGA core owns; the NV extension
/// registers sit above it — Bochs geforce.h `VGA_CRTC_MAX`.
const VGA_CRTC_MAX: usize = 0x18;
const GEFORCE_CRTC_MAX: usize = 0xF0;
const GEFORCE_CHANNEL_COUNT: usize = 32;
const GEFORCE_SUBCHANNEL_COUNT: usize = 8;
const GEFORCE_CACHE1_SIZE: usize = 64;
/// Size of the BAR0 register window — Bochs geforce.cc `GEFORCE_PNPMMIO_SIZE`.
const GEFORCE_PNPMMIO_SIZE: u32 = 0x0100_0000;
const BX_ROP_PATTERN: u8 = 0x01;

// The ports Bochs geforce.cc `svga_read`/`svga_write` answer before the
// standard VGA does.
const PORT_CRTC_INDEX_MONO: u16 = 0x03B4;
const PORT_CRTC_DATA_MONO: u16 = 0x03B5;
const PORT_INPUT_STATUS_0: u16 = 0x03C2;
const PORT_VGA_ENABLE: u16 = 0x03C3;
const PORT_RMA_LOW: u16 = 0x03D0;
const PORT_RMA_HIGH: u16 = 0x03D2;
const PORT_CRTC_INDEX: u16 = 0x03D4;
const PORT_CRTC_DATA: u16 = 0x03D5;

/// CRTC indices whose write changes the extended mode — Bochs geforce.cc
/// `svga_write` sets `svga_needs_update_mode` for exactly these.
const CRTC_MODE_UPDATE_INDICES: [u8; 14] = [
    0x01, 0x07, 0x09, 0x0c, 0x0d, 0x12, 0x13, 0x15, 0x19, 0x25, 0x28, 0x2D, 0x41, 0x42,
];

/// Where a Real Mode Access data transfer lands. Bochs geforce.cc
/// `svga_read`/`svga_write` take bit 31 of `rma_addr` to choose VRAM over the
/// register file, and bound each by its own size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RmaTarget {
    /// An offset into the BAR0 register file, below `GEFORCE_PNPMMIO_SIZE`.
    Register(u32),
    /// An offset into video memory, below `memsize`.
    Vram(u32),
}

/// The register a BAR0 access lands on — Bochs geforce.cc
/// `geforce_mem_read_handler` takes the address modulo the window.
fn bar0_offset(at: WindowOffset) -> u32 {
    (at.get() & u64::from(GEFORCE_PNPMMIO_SIZE - 1)) as u32
}

/// Copy an access's bytes into the caller's buffer, as far as both reach.
fn fill_access(data: &mut [u8], bytes: &[u8]) {
    for (slot, byte) in data.iter_mut().zip(bytes) {
        *slot = *byte;
    }
}

/// The first `N` bytes of a write, little-endian; a byte the caller did not
/// supply reads as zero.
fn access_bytes<const N: usize>(data: &[u8]) -> [u8; N] {
    core::array::from_fn(|index| data.get(index).copied().unwrap_or(0))
}

fn align_up(x: u32, a: u32) -> u32 {
    (x + a - 1) & !(a - 1)
}

/// GeForce model identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GeForceModel {
    GeForce2 = 0,
    GeForce3 = 1,
    GeForceFx5900 = 2,
    GeForce6800 = 3,
}

impl GeForceModel {
    pub fn card_type(self) -> u32 {
        match self {
            Self::GeForce2 => 0x15,
            Self::GeForce3 => 0x20,
            Self::GeForceFx5900 => 0x35,
            Self::GeForce6800 => 0x40,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::GeForce2 => "GeForce2 Pro",
            Self::GeForce3 => "GeForce3 Ti 500",
            Self::GeForceFx5900 => "GeForce FX 5900",
            Self::GeForce6800 => "GeForce 6800 GT",
        }
    }
}

// ---------------------------------------------------------------------------
// Texture state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct GfTexture {
    pub offset: u32,
    pub dma_obj: u32,
    pub format: u32,
    pub cubemap: bool,
    pub linear: bool,
    pub unnormalized: bool,
    pub compressed: bool,
    pub dxt_alpha_data: bool,
    pub dxt_alpha_explicit: bool,
    pub color_bytes: u32,
    pub levels: u32,
    pub base_size: [u32; 3],
    pub size: [u32; 3],
    pub face_bytes: u32,
    pub wrap: [u32; 3],
    pub control0: u32,
    pub enabled: bool,
    pub control1: u32,
    pub signed_any: bool,
    pub signed_comp: [bool; 4],
    pub image_rect: u32,
    pub pal_dma_obj: u32,
    pub pal_ofs: u32,
    pub control3: u32,
    pub key_color: u32,
    pub offset_matrix: [f32; 4],
}

impl Default for GfTexture {
    fn default() -> Self {
        Self {
            offset: 0,
            dma_obj: 0,
            format: 0,
            cubemap: false,
            linear: false,
            unnormalized: false,
            compressed: false,
            dxt_alpha_data: false,
            dxt_alpha_explicit: false,
            color_bytes: 1,
            levels: 0,
            base_size: [0; 3],
            size: [0; 3],
            face_bytes: 0,
            wrap: [0; 3],
            control0: 0,
            enabled: false,
            control1: 0,
            signed_any: false,
            signed_comp: [false; 4],
            image_rect: 0,
            pal_dma_obj: 0,
            pal_ofs: 0,
            control3: 0,
            key_color: 0,
            offset_matrix: [0.0; 4],
        }
    }
}

// ---------------------------------------------------------------------------
// Light state
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct GfLight {
    pub ambient_color: [f32; 3],
    pub diffuse_color: [f32; 3],
    pub specular_color: [f32; 3],
    pub inf_half_vector: [f32; 3],
    pub inf_direction: [f32; 3],
    pub spot_direction: [f32; 4],
    pub local_position: [f32; 3],
    pub local_attenuation: [f32; 3],
}

// ---------------------------------------------------------------------------
// Subchannel / DMA state
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct GfSubchannel {
    pub object: u32,
    pub engine: u8,
    pub notifier: u32,
}

#[derive(Clone, Default)]
pub struct GfDmaState {
    pub mthd: u32,
    pub subc: u32,
    pub mcnt: u32,
    pub ni: bool,
}

// ---------------------------------------------------------------------------
// Per-channel state
// ---------------------------------------------------------------------------

/// Per-channel GPU state encompassing 2D, 3D, and DMA FIFO state.
pub struct GfChannel {
    pub subr_return: u32,
    pub subr_active: bool,
    pub dma_state: GfDmaState,
    pub schs: [GfSubchannel; GEFORCE_SUBCHANNEL_COUNT],
    pub notify_pending: bool,
    pub notify_type: u32,

    // Surface-to-surface 2D
    pub s2d_locked: bool,
    pub s2d_img_src: u32,
    pub s2d_img_dst: u32,
    pub s2d_color_fmt: u32,
    pub s2d_color_bytes: u32,
    pub s2d_pitch_src: u32,
    pub s2d_pitch_dst: u32,
    pub s2d_ofs_src: u32,
    pub s2d_ofs_dst: u32,

    // Swizzled surface
    pub swzs_img_obj: u32,
    pub swzs_fmt: u32,
    pub swzs_color_bytes: u32,
    pub swzs_width: u32,
    pub swzs_height: u32,
    pub swzs_ofs: u32,

    // Image-from-CPU
    pub ifc_color_key_enable: bool,
    pub ifc_clip_enable: bool,
    pub ifc_operation: u32,
    pub ifc_color_fmt: u32,
    pub ifc_color_bytes: u32,
    pub ifc_pixels_per_word: u32,
    pub ifc_x: u32,
    pub ifc_y: u32,
    pub ifc_ofs_x: u32,
    pub ifc_ofs_y: u32,
    pub ifc_draw_offset: u32,
    pub ifc_redraw_offset: u32,
    pub ifc_dst_width: u32,
    pub ifc_dst_height: u32,
    pub ifc_src_width: u32,
    pub ifc_src_height: u32,
    pub ifc_clip_x0: u32,
    pub ifc_clip_y0: u32,
    pub ifc_clip_x1: u32,
    pub ifc_clip_y1: u32,

    // Indexed image from CPU
    pub iifc_palette: u32,
    pub iifc_palette_ofs: u32,
    pub iifc_operation: u32,
    pub iifc_color_fmt: u32,
    pub iifc_color_bytes: u32,
    pub iifc_bpp4: u32,
    pub iifc_yx: u32,
    pub iifc_dhw: u32,
    pub iifc_shw: u32,
    pub iifc_words_ptr: u32,
    pub iifc_words_left: u32,
    pub iifc_words: Option<Vec<u32>>,

    // Scaled image from CPU
    pub sifc_operation: u32,
    pub sifc_color_fmt: u32,
    pub sifc_color_bytes: u32,
    pub sifc_shw: u32,
    pub sifc_dxds: u32,
    pub sifc_dydt: u32,
    pub sifc_clip_yx: u32,
    pub sifc_clip_hw: u32,
    pub sifc_syx: u32,
    pub sifc_words_ptr: u32,
    pub sifc_words_left: u32,
    pub sifc_words: Option<Vec<u32>>,

    // Blit
    pub blit_color_key_enable: bool,
    pub blit_operation: u32,
    pub blit_syx: u32,
    pub blit_dyx: u32,
    pub blit_hw: u32,

    // Texture-from-CPU
    pub tfc_swizzled: bool,
    pub tfc_color_fmt: u32,
    pub tfc_color_bytes: u32,
    pub tfc_yx: u32,
    pub tfc_hw: u32,
    pub tfc_clip_wx: u32,
    pub tfc_clip_hy: u32,
    pub tfc_words_ptr: u32,
    pub tfc_words_left: u32,
    pub tfc_words: Option<Vec<u32>>,
    pub tfc_upload: bool,
    pub tfc_upload_offset: u32,

    // Scaled image from memory
    pub sifm_src: u32,
    pub sifm_swizzled: bool,
    pub sifm_swizzled_0389: bool,
    pub sifm_operation: u32,
    pub sifm_color_fmt: u32,
    pub sifm_color_bytes: u32,
    pub sifm_syx: u32,
    pub sifm_dyx: u32,
    pub sifm_shw: u32,
    pub sifm_dhw: u32,
    pub sifm_dudx: i32,
    pub sifm_dvdy: i32,
    pub sifm_sfmt: u32,
    pub sifm_sofs: u32,

    // Memory-to-memory format
    pub m2mf_src: u32,
    pub m2mf_dst: u32,
    pub m2mf_src_offset: u32,
    pub m2mf_dst_offset: u32,
    pub m2mf_src_pitch: u32,
    pub m2mf_dst_pitch: u32,
    pub m2mf_line_length: u32,
    pub m2mf_line_count: u32,
    pub m2mf_format: u32,
    pub m2mf_buffer_notify: u32,

    // 3D state - objects
    pub d3d_a_obj: u32,
    pub d3d_b_obj: u32,
    pub d3d_color_obj: u32,
    pub d3d_zeta_obj: u32,
    pub d3d_vertex_a_obj: u32,
    pub d3d_vertex_b_obj: u32,
    pub d3d_report_obj: u32,

    // 3D state - surface/clip
    pub d3d_clip_horizontal: u32,
    pub d3d_clip_vertical: u32,
    pub d3d_surface_format: u32,
    pub d3d_color_bytes: u32,
    pub d3d_depth_bytes: u32,
    pub d3d_surface_pitch_a: u32,
    pub d3d_surface_pitch_z: u32,
    pub d3d_surface_color_offset: u32,
    pub d3d_surface_zeta_offset: u32,

    // 3D state - lighting/material
    pub d3d_local_viewer: bool,
    pub d3d_color_material_emission: u32,
    pub d3d_color_material_ambient: u32,
    pub d3d_color_material_diffuse: u32,
    pub d3d_color_material_specular: u32,
    pub d3d_lighting_enable: u32,
    pub d3d_normalize_enable: u32,
    pub d3d_material_factor: [f32; 4],
    pub d3d_separate_specular: u32,
    pub d3d_light_enable_mask: u32,
    pub d3d_specular_params: [f32; 6],
    pub d3d_specular_power: f32,
    pub d3d_scene_ambient_color: [f32; 4],
    pub d3d_eye_position: [f32; 4],
    pub d3d_light: [GfLight; 8],

    // 3D state - fog
    pub d3d_fog_mode: u32,
    pub d3d_fog_gen_mode: u32,
    pub d3d_fog_params: [f32; 3],
    pub d3d_fog_enable: u32,
    pub d3d_fog_color: [f32; 4],

    // 3D state - window/viewport
    pub d3d_window_offset_x: i16,
    pub d3d_window_offset_y: i16,
    pub d3d_window_clip_x1: [u32; 8],
    pub d3d_window_clip_x2: [u32; 8],
    pub d3d_window_clip_y1: [u32; 8],
    pub d3d_window_clip_y2: [u32; 8],
    pub d3d_viewport_x: u32,
    pub d3d_viewport_width: u32,
    pub d3d_viewport_y: u32,
    pub d3d_viewport_height: u32,
    pub d3d_viewport_offset: [f32; 4],
    pub d3d_viewport_scale: [f32; 4],
    pub d3d_scissor_x: u32,
    pub d3d_scissor_width: u32,
    pub d3d_scissor_y: u32,
    pub d3d_scissor_height: u32,

    // 3D state - blending
    pub d3d_alpha_test_enable: u32,
    pub d3d_alpha_func: u32,
    pub d3d_alpha_ref: u32,
    pub d3d_blend_enable: u32,
    pub d3d_blend_sfactor_rgb: u16,
    pub d3d_blend_sfactor_alpha: u16,
    pub d3d_blend_dfactor_rgb: u16,
    pub d3d_blend_dfactor_alpha: u16,
    pub d3d_blend_equation_rgb: u16,
    pub d3d_blend_equation_alpha: u16,
    pub d3d_blend_color: [f32; 4],

    // 3D state - depth/stencil
    pub d3d_depth_test_enable: u32,
    pub d3d_depth_write_enable: u32,
    pub d3d_depth_func: u32,
    pub d3d_stencil_test_enable: u32,
    pub d3d_stencil_mask: u32,
    pub d3d_stencil_func: u32,
    pub d3d_stencil_func_ref: u32,
    pub d3d_stencil_func_mask: u32,
    pub d3d_stencil_op_sfail: u32,
    pub d3d_stencil_op_dpfail: u32,
    pub d3d_stencil_op_dppass: u32,

    // 3D state - rasterization
    pub d3d_cull_face_enable: u32,
    pub d3d_cull_face: u32,
    pub d3d_front_face: u32,
    pub d3d_color_mask: u32,
    pub d3d_shade_mode: u32,
    pub d3d_clip_min: f32,
    pub d3d_clip_max: f32,

    // 3D state - register combiners
    pub d3d_combiner_alpha_icw: [u32; 8],
    pub d3d_combiner_final: [u32; 2],
    pub d3d_combiner_const_color: [[[f32; 4]; 2]; 8],
    pub d3d_combiner_alpha_ocw: [u32; 8],
    pub d3d_combiner_color_icw: [u32; 8],
    pub d3d_combiner_color_ocw: [u32; 8],
    pub d3d_combiner_control: u32,
    pub d3d_combiner_control_num_stages: u32,

    // 3D state - textures/shaders
    pub d3d_texture: [GfTexture; 16],
    pub d3d_tex_shader_op: [u32; 4],
    pub d3d_tex_shader_previous: [u32; 4],
    pub d3d_shader_program: u32,
    pub d3d_shader_obj: u32,
    pub d3d_shader_offset: u32,
    pub d3d_shader_control: u32,

    // 3D state - transforms
    pub d3d_transform_execution_mode: u32,
    pub d3d_transform_program_load: u32,
    pub d3d_transform_program_start: u32,
    pub d3d_transform_constant_load: u32,
    pub d3d_view_matrix_enable: u32,
    pub d3d_model_view_matrix: [[f32; 16]; 2],
    pub d3d_inverse_model_view_matrix: [f32; 12],
    pub d3d_composite_matrix: [f32; 16],
    pub d3d_texture_matrix: [[f32; 16]; 8],
    pub d3d_texture_matrix_enable: [u32; 16],
    pub d3d_texgen: [[u32; 4]; 8],
    pub d3d_texgen_plane: [[[f32; 4]; 4]; 8],
    pub d3d_transform_program: Box<[[u32; 4]; 544]>,
    pub d3d_transform_constant: Box<[[f32; 4]; 512]>,

    // 3D state - vertex processing
    pub d3d_attrib_count: u32,
    pub d3d_vertex_data_base_index: u32,
    pub d3d_vertex_data_array_offset: [u32; 16],
    pub d3d_vertex_data_array_format_type: [u32; 16],
    pub d3d_vertex_data_array_format_size: [u32; 16],
    pub d3d_vertex_data_array_format_stride: [u32; 16],
    pub d3d_vertex_data_array_format_dx: [bool; 16],
    pub d3d_vertex_data_array_format_homogeneous: [bool; 16],
    pub d3d_begin_end: u32,
    pub d3d_primitive_done: bool,
    pub d3d_triangle_flip: bool,
    pub d3d_vertex_index: u32,
    pub d3d_attrib_index: u32,
    pub d3d_comp_index: u32,
    pub d3d_vertex_data: [[[f32; 4]; 16]; 4],
    pub d3d_vertex_data_imm: [[f32; 4]; 16],
    pub d3d_index_array_offset: u32,
    pub d3d_index_array_dma: u32,
    pub d3d_attrib_in_normal: u32,
    pub d3d_attrib_in_color: [u32; 2],
    pub d3d_attrib_out_color: [u32; 2],
    pub d3d_attrib_out_fogc: u32,
    pub d3d_attrib_in_tex_coord: [u32; 16],
    pub d3d_attrib_out_tex_coord: [u32; 16],
    pub d3d_attrib_out_enable: [bool; 32],
    pub d3d_vs_temp_regs_count: u32,
    pub d3d_tex_coord_count: u32,

    // 3D state - semaphore/clear
    pub d3d_semaphore_obj: u32,
    pub d3d_semaphore_offset: u32,
    pub d3d_zstencil_clear_value: u32,
    pub d3d_color_clear_value: u32,
    pub d3d_clear_surface: u32,

    // ROP / beta / clip / chroma / pattern
    pub rop: u8,
    pub beta: u32,
    pub clip_x: u16,
    pub clip_y: u16,
    pub clip_width: u16,
    pub clip_height: u16,
    pub chroma_color_fmt: u32,
    pub chroma_color: u32,
    pub patt_shape: u32,
    pub patt_type_color: bool,
    pub patt_bg_color: u32,
    pub patt_fg_color: u32,
    pub patt_data_mono: [bool; 64],
    pub patt_data_color: [u32; 64],

    // GDI rectangle text
    pub gdi_operation: u32,
    pub gdi_color_fmt: u32,
    pub gdi_mono_fmt: u32,
    pub gdi_clip_yx0: u32,
    pub gdi_clip_yx1: u32,
    pub gdi_rect_color: u32,
    pub gdi_rect_xy: u32,
    pub gdi_rect_yx0: u32,
    pub gdi_rect_yx1: u32,
    pub gdi_rect_wh: u32,
    pub gdi_bg_color: u32,
    pub gdi_fg_color: u32,
    pub gdi_image_swh: u32,
    pub gdi_image_dwh: u32,
    pub gdi_image_xy: u32,
    pub gdi_words_ptr: u32,
    pub gdi_words_left: u32,
    pub gdi_words: Option<Vec<u32>>,

    // Rectangle
    pub rect_operation: u32,
    pub rect_color_fmt: u32,
    pub rect_color: u32,
    pub rect_yx: u32,
    pub rect_hw: u32,
}

impl GfChannel {
    pub fn new() -> Self {
        Self {
            subr_return: 0,
            subr_active: false,
            dma_state: GfDmaState::default(),
            schs: core::array::from_fn(|_| GfSubchannel::default()),
            notify_pending: false,
            notify_type: 0,
            s2d_locked: false,
            s2d_img_src: 0,
            s2d_img_dst: 0,
            s2d_color_fmt: 0,
            s2d_color_bytes: 1,
            s2d_pitch_src: 0,
            s2d_pitch_dst: 0,
            s2d_ofs_src: 0,
            s2d_ofs_dst: 0,
            swzs_img_obj: 0,
            swzs_fmt: 0,
            swzs_color_bytes: 1,
            swzs_width: 0,
            swzs_height: 0,
            swzs_ofs: 0,
            ifc_color_key_enable: false,
            ifc_clip_enable: false,
            ifc_operation: 0,
            ifc_color_fmt: 0,
            ifc_color_bytes: 4,
            ifc_pixels_per_word: 1,
            ifc_x: 0,
            ifc_y: 0,
            ifc_ofs_x: 0,
            ifc_ofs_y: 0,
            ifc_draw_offset: 0,
            ifc_redraw_offset: 0,
            ifc_dst_width: 0,
            ifc_dst_height: 0,
            ifc_src_width: 0,
            ifc_src_height: 0,
            ifc_clip_x0: 0,
            ifc_clip_y0: 0,
            ifc_clip_x1: 0,
            ifc_clip_y1: 0,
            iifc_palette: 0,
            iifc_palette_ofs: 0,
            iifc_operation: 0,
            iifc_color_fmt: 0,
            iifc_color_bytes: 4,
            iifc_bpp4: 0,
            iifc_yx: 0,
            iifc_dhw: 0,
            iifc_shw: 0,
            iifc_words_ptr: 0,
            iifc_words_left: 0,
            iifc_words: None,
            sifc_operation: 0,
            sifc_color_fmt: 0,
            sifc_color_bytes: 4,
            sifc_shw: 0,
            sifc_dxds: 0,
            sifc_dydt: 0,
            sifc_clip_yx: 0,
            sifc_clip_hw: 0,
            sifc_syx: 0,
            sifc_words_ptr: 0,
            sifc_words_left: 0,
            sifc_words: None,
            blit_color_key_enable: false,
            blit_operation: 0,
            blit_syx: 0,
            blit_dyx: 0,
            blit_hw: 0,
            tfc_swizzled: false,
            tfc_color_fmt: 0,
            tfc_color_bytes: 4,
            tfc_yx: 0,
            tfc_hw: 0,
            tfc_clip_wx: 0,
            tfc_clip_hy: 0,
            tfc_words_ptr: 0,
            tfc_words_left: 0,
            tfc_words: None,
            tfc_upload: false,
            tfc_upload_offset: 0,
            sifm_src: 0,
            sifm_swizzled: false,
            sifm_swizzled_0389: false,
            sifm_operation: 0,
            sifm_color_fmt: 0,
            sifm_color_bytes: 4,
            sifm_syx: 0,
            sifm_dyx: 0,
            sifm_shw: 0,
            sifm_dhw: 0,
            sifm_dudx: 0,
            sifm_dvdy: 0,
            sifm_sfmt: 0,
            sifm_sofs: 0,
            m2mf_src: 0,
            m2mf_dst: 0,
            m2mf_src_offset: 0,
            m2mf_dst_offset: 0,
            m2mf_src_pitch: 0,
            m2mf_dst_pitch: 0,
            m2mf_line_length: 0,
            m2mf_line_count: 0,
            m2mf_format: 0,
            m2mf_buffer_notify: 0,
            d3d_a_obj: 0,
            d3d_b_obj: 0,
            d3d_color_obj: 0,
            d3d_zeta_obj: 0,
            d3d_vertex_a_obj: 0,
            d3d_vertex_b_obj: 0,
            d3d_report_obj: 0,
            d3d_clip_horizontal: 0,
            d3d_clip_vertical: 0,
            d3d_surface_format: 0,
            d3d_color_bytes: 1,
            d3d_depth_bytes: 1,
            d3d_surface_pitch_a: 0,
            d3d_surface_pitch_z: 0,
            d3d_surface_color_offset: 0,
            d3d_surface_zeta_offset: 0,
            d3d_local_viewer: false,
            d3d_color_material_emission: 0,
            d3d_color_material_ambient: 0,
            d3d_color_material_diffuse: 0,
            d3d_color_material_specular: 0,
            d3d_lighting_enable: 0,
            d3d_normalize_enable: 0,
            d3d_material_factor: [0.0; 4],
            d3d_separate_specular: 0,
            d3d_light_enable_mask: 0,
            d3d_specular_params: [0.0; 6],
            d3d_specular_power: 0.0,
            d3d_scene_ambient_color: [0.0; 4],
            d3d_eye_position: [0.0; 4],
            d3d_light: core::array::from_fn(|_| GfLight::default()),
            d3d_fog_mode: 0,
            d3d_fog_gen_mode: 0,
            d3d_fog_params: [0.0; 3],
            d3d_fog_enable: 0,
            d3d_fog_color: [0.0; 4],
            d3d_window_offset_x: 0,
            d3d_window_offset_y: 0,
            d3d_window_clip_x1: [0; 8],
            d3d_window_clip_x2: [0; 8],
            d3d_window_clip_y1: [0; 8],
            d3d_window_clip_y2: [0; 8],
            d3d_viewport_x: 0,
            d3d_viewport_width: 0,
            d3d_viewport_y: 0,
            d3d_viewport_height: 0,
            d3d_viewport_offset: [0.0; 4],
            d3d_viewport_scale: [0.0; 4],
            d3d_scissor_x: 0,
            d3d_scissor_width: 0,
            d3d_scissor_y: 0,
            d3d_scissor_height: 0,
            d3d_alpha_test_enable: 0,
            d3d_alpha_func: 0,
            d3d_alpha_ref: 0,
            d3d_blend_enable: 0,
            d3d_blend_sfactor_rgb: 0,
            d3d_blend_sfactor_alpha: 0,
            d3d_blend_dfactor_rgb: 0,
            d3d_blend_dfactor_alpha: 0,
            d3d_blend_equation_rgb: 0,
            d3d_blend_equation_alpha: 0,
            d3d_blend_color: [0.0; 4],
            d3d_depth_test_enable: 0,
            d3d_depth_write_enable: 0,
            d3d_depth_func: 0,
            d3d_stencil_test_enable: 0,
            d3d_stencil_mask: 0,
            d3d_stencil_func: 0,
            d3d_stencil_func_ref: 0,
            d3d_stencil_func_mask: 0,
            d3d_stencil_op_sfail: 0,
            d3d_stencil_op_dpfail: 0,
            d3d_stencil_op_dppass: 0,
            d3d_cull_face_enable: 0,
            d3d_cull_face: 0,
            d3d_front_face: 0,
            d3d_color_mask: 0,
            d3d_shade_mode: 0,
            d3d_clip_min: 0.0,
            d3d_clip_max: 0.0,
            d3d_combiner_alpha_icw: [0; 8],
            d3d_combiner_final: [0; 2],
            d3d_combiner_const_color: [[[0.0; 4]; 2]; 8],
            d3d_combiner_alpha_ocw: [0; 8],
            d3d_combiner_color_icw: [0; 8],
            d3d_combiner_color_ocw: [0; 8],
            d3d_combiner_control: 0,
            d3d_combiner_control_num_stages: 0,
            d3d_texture: core::array::from_fn(|_| GfTexture::default()),
            d3d_tex_shader_op: [0; 4],
            d3d_tex_shader_previous: [0; 4],
            d3d_shader_program: 0,
            d3d_shader_obj: 0,
            d3d_shader_offset: 0,
            d3d_shader_control: 0,
            d3d_transform_execution_mode: 0,
            d3d_transform_program_load: 0,
            d3d_transform_program_start: 0,
            d3d_transform_constant_load: 0,
            d3d_view_matrix_enable: 0,
            d3d_model_view_matrix: [[0.0; 16]; 2],
            d3d_inverse_model_view_matrix: [0.0; 12],
            d3d_composite_matrix: [0.0; 16],
            d3d_texture_matrix: [[0.0; 16]; 8],
            d3d_texture_matrix_enable: [0; 16],
            d3d_texgen: [[0; 4]; 8],
            d3d_texgen_plane: [[[0.0; 4]; 4]; 8],
            d3d_transform_program: Box::new([[0u32; 4]; 544]),
            d3d_transform_constant: Box::new([[0.0f32; 4]; 512]),
            d3d_attrib_count: 0,
            d3d_vertex_data_base_index: 0,
            d3d_vertex_data_array_offset: [0; 16],
            d3d_vertex_data_array_format_type: [0; 16],
            d3d_vertex_data_array_format_size: [0; 16],
            d3d_vertex_data_array_format_stride: [0; 16],
            d3d_vertex_data_array_format_dx: [false; 16],
            d3d_vertex_data_array_format_homogeneous: [false; 16],
            d3d_begin_end: 0,
            d3d_primitive_done: false,
            d3d_triangle_flip: false,
            d3d_vertex_index: 0,
            d3d_attrib_index: 0,
            d3d_comp_index: 0,
            d3d_vertex_data: [[[0.0; 4]; 16]; 4],
            d3d_vertex_data_imm: [[0.0; 4]; 16],
            d3d_index_array_offset: 0,
            d3d_index_array_dma: 0,
            d3d_attrib_in_normal: 0,
            d3d_attrib_in_color: [0; 2],
            d3d_attrib_out_color: [0; 2],
            d3d_attrib_out_fogc: 0,
            d3d_attrib_in_tex_coord: [0; 16],
            d3d_attrib_out_tex_coord: [0; 16],
            d3d_attrib_out_enable: [false; 32],
            d3d_vs_temp_regs_count: 0,
            d3d_tex_coord_count: 0,
            d3d_semaphore_obj: 0,
            d3d_semaphore_offset: 0,
            d3d_zstencil_clear_value: 0,
            d3d_color_clear_value: 0,
            d3d_clear_surface: 0,
            rop: 0,
            beta: 0,
            clip_x: 0,
            clip_y: 0,
            clip_width: 0,
            clip_height: 0,
            chroma_color_fmt: 0,
            chroma_color: 0,
            patt_shape: 0,
            patt_type_color: false,
            patt_bg_color: 0,
            patt_fg_color: 0,
            patt_data_mono: [false; 64],
            patt_data_color: [0; 64],
            gdi_operation: 0,
            gdi_color_fmt: 0,
            gdi_mono_fmt: 0,
            gdi_clip_yx0: 0,
            gdi_clip_yx1: 0,
            gdi_rect_color: 0,
            gdi_rect_xy: 0,
            gdi_rect_yx0: 0,
            gdi_rect_yx1: 0,
            gdi_rect_wh: 0,
            gdi_bg_color: 0,
            gdi_fg_color: 0,
            gdi_image_swh: 0,
            gdi_image_dwh: 0,
            gdi_image_xy: 0,
            gdi_words_ptr: 0,
            gdi_words_left: 0,
            gdi_words: None,
            rect_operation: 0,
            rect_color_fmt: 0,
            rect_color: 0,
            rect_yx: 0,
            rect_hw: 0,
        }
    }
}

impl Default for GfChannel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Hardware cursor state
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct HwCursor {
    pub vram: bool,
    pub offset: u32,
    pub x: i16,
    pub y: i16,
    pub size: u8,
    pub bpp32: bool,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// CRTC register block
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CrtcRegs {
    pub index: u8,
    pub reg: [u8; GEFORCE_CRTC_MAX + 1],
}

impl Default for CrtcRegs {
    fn default() -> Self {
        Self {
            index: (GEFORCE_CRTC_MAX + 1) as u8,
            reg: [0u8; GEFORCE_CRTC_MAX + 1],
        }
    }
}

// ---------------------------------------------------------------------------
// ROP handler type
// ---------------------------------------------------------------------------

/// Binary forward ROP function pointer type.
/// (dst, src, unused1, unused2, byte_count, pixel_count)
pub type RopHandler = fn(&mut [u8], &[u8], usize, usize, u32, u32);

/// No-op ROP: destination unchanged
fn rop_nop(_dst: &mut [u8], _src: &[u8], _: usize, _: usize, _cb: u32, _pc: u32) {}

/// Copy ROP: destination = source
fn rop_src(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    let n = cb as usize;
    dst[..n].copy_from_slice(&src[..n]);
}

/// Clear ROP: destination = 0
fn rop_0(dst: &mut [u8], _src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for b in &mut dst[..cb as usize] {
        *b = 0;
    }
}

/// Set ROP: destination = 0xFF
fn rop_1(dst: &mut [u8], _src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for b in &mut dst[..cb as usize] {
        *b = 0xFF;
    }
}

fn rop_notdst(dst: &mut [u8], _src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for b in &mut dst[..cb as usize] {
        *b = !*b;
    }
}

fn rop_src_and_dst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] &= src[i];
    }
}

fn rop_notsrc_and_dst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] &= !src[i];
    }
}

fn rop_src_and_notdst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] = src[i] & !dst[i];
    }
}

fn rop_notsrc(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] = !src[i];
    }
}

fn rop_src_xor_dst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] ^= src[i];
    }
}

fn rop_notsrc_or_notdst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] = !src[i] | !dst[i];
    }
}

fn rop_src_or_dst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] |= src[i];
    }
}

fn rop_notsrc_or_dst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] = !src[i] | dst[i];
    }
}

fn rop_src_notxor_dst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] = !(src[i] ^ dst[i]);
    }
}

fn rop_notsrc_and_notdst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] = !src[i] & !dst[i];
    }
}

fn rop_src_or_notdst(dst: &mut [u8], src: &[u8], _: usize, _: usize, cb: u32, _pc: u32) {
    for i in 0..cb as usize {
        dst[i] = src[i] | !dst[i];
    }
}

/// Ternary ROP: applies pattern-based operation
#[allow(
    dead_code,
    reason = "Bochs bitblt.h `bx_ternary_rop`, reached from `pixel_operation`, which is ported ahead of its caller"
)]
fn bx_ternary_rop(rop: u8, dst: &mut [u8], src: &[u8], pat: &[u8], cb: u32) {
    for i in 0..cb as usize {
        let mut result = 0u8;
        for bit in 0..8u8 {
            let s = (src[i] >> bit) & 1;
            let d = (dst[i] >> bit) & 1;
            let p = (pat[i] >> bit) & 1;
            let idx = (p << 2) | (s << 1) | d;
            result |= ((rop >> idx) & 1) << bit;
        }
        dst[i] = result;
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn uint32_as_float(val: u32) -> f32 {
    f32::from_bits(val)
}

#[allow(
    dead_code,
    reason = "Bochs geforce.cc `alpha_wrap`, reached from `pixel_operation`, which is ported ahead of its caller"
)]
fn alpha_wrap(value: i32) -> u8 {
    (-(value >> 8) ^ value) as u8
}

#[allow(dead_code, reason = "Bochs geforce.cc `color_565_to_888`, ported ahead of its caller")]
fn color_565_to_888(value: u16) -> u32 {
    let r = ((value >> 11) & 0x1F) as u32;
    let g = ((value >> 5) & 0x3F) as u32;
    let b = (value & 0x1F) as u32;
    let r8 = (r << 3) | (r >> 2);
    let g8 = (g << 2) | (g >> 4);
    let b8 = (b << 3) | (b >> 2);
    (r8 << 16) | (g8 << 8) | b8
}

#[allow(dead_code, reason = "Bochs geforce.cc `color_888_to_565`, ported ahead of its caller")]
fn color_888_to_565(value: u32) -> u16 {
    let r = ((value >> 19) & 0x1F) as u16;
    let g = ((value >> 10) & 0x3F) as u16;
    let b = ((value >> 3) & 0x1F) as u16;
    (r << 11) | (g << 5) | b
}

#[allow(dead_code, reason = "Bochs geforce.cc `dot3`, ported ahead of its caller")]
fn dot3(x: &[f32; 3], y: &[f32; 3]) -> f32 {
    x[0] * y[0] + x[1] * y[1] + x[2] * y[2]
}

#[allow(
    dead_code,
    reason = "Bochs geforce.cc `dot3` applied to a row of a `float[4]` table, ported ahead of its caller"
)]
fn dot3_slice(x: &[f32], y: &[f32]) -> f32 {
    x[0] * y[0] + x[1] * y[1] + x[2] * y[2]
}

#[allow(dead_code, reason = "Bochs geforce.cc `dot4`, ported ahead of its caller")]
fn dot4(x: &[f32], y: &[f32]) -> f32 {
    x[0] * y[0] + x[1] * y[1] + x[2] * y[2] + x[3] * y[3]
}

#[allow(dead_code, reason = "Bochs geforce.cc `length`, ported ahead of its caller")]
fn vec3_length(v: &[f32; 3]) -> f32 {
    // Named through the trait so every build takes the same path: a test
    // build links std, whose inherent `f32::sqrt` would otherwise win the
    // method lookup and leave production's `no_std` square root untested.
    FloatExt::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}

#[allow(
    dead_code,
    reason = "Bochs geforce.cc `normalize(float v[3])`, ported ahead of its caller"
)]
fn vec3_normalize(v: &mut [f32; 3]) -> f32 {
    let l = vec3_length(v);
    let s = 1.0 / l;
    v[0] *= s;
    v[1] *= s;
    v[2] *= s;
    l
}

#[allow(
    dead_code,
    reason = "Bochs geforce.cc `normalize(float in[3], float out[3])`, ported ahead of its caller"
)]
fn vec3_normalize_into(input: &[f32; 3], out: &mut [f32; 3]) {
    let s = 1.0 / vec3_length(input);
    out[0] = input[0] * s;
    out[1] = input[1] * s;
    out[2] = input[2] * s;
}

#[allow(dead_code, reason = "Bochs geforce.cc `edge_function`, ported ahead of its caller")]
fn edge_function(v0: &[f32; 4], v1: &[f32; 4], v2: &[f32]) -> f64 {
    ((v1[0] - v0[0]) as f64) * ((v2[1] - v0[1]) as f64)
        - ((v1[1] - v0[1]) as f64) * ((v2[0] - v0[0]) as f64)
}

#[allow(dead_code, reason = "Bochs geforce.cc `compare`, ported ahead of its caller")]
fn compare(func: u32, val1: u32, val2: u32) -> bool {
    match func {
        1 | 0x200 => false,        // NEVER
        2 | 0x201 => val1 < val2,  // LESS
        3 | 0x202 => val1 == val2, // EQUAL
        4 | 0x203 => val1 <= val2, // LEQUAL
        5 | 0x204 => val1 > val2,  // GREATER
        6 | 0x205 => val1 != val2, // NOTEQUAL
        7 | 0x206 => val1 >= val2, // GEQUAL
        8 | 0x207 => true,         // ALWAYS
        _ => val1 < val2,          // default LESS
    }
}

#[allow(dead_code, reason = "Bochs geforce.cc `blend_factor`, ported ahead of its caller")]
fn blend_factor(
    factor: u16,
    src_rgb: f32,
    src_a: f32,
    dst_rgb: f32,
    dst_a: f32,
    const_rgb: f32,
    const_a: f32,
) -> f32 {
    match factor {
        0x0000 | 0x1001 => 0.0,                    // ZERO
        0x0001 | 0x1002 => 1.0,                    // ONE
        0x0300 | 0x1003 => src_rgb,                // SRC_COLOR
        0x0301 | 0x1004 => 1.0 - src_rgb,          // INV_SRC_COLOR
        0x0302 | 0x1005 => src_a,                  // SRC_ALPHA
        0x0303 | 0x1006 => 1.0 - src_a,            // INV_SRC_ALPHA
        0x0304 | 0x1007 => dst_a,                  // DEST_ALPHA
        0x0305 | 0x1008 => 1.0 - dst_a,            // INV_DEST_ALPHA
        0x0306 | 0x1009 => dst_rgb,                // DEST_COLOR
        0x0307 | 0x100a => 1.0 - dst_rgb,          // INV_DEST_COLOR
        0x0308 | 0x100b => src_a.min(1.0 - dst_a), // SRC_ALPHA_SAT
        0x8001 | 0x100e => const_rgb,              // CONSTANT_COLOR
        0x8002 | 0x100f => 1.0 - const_rgb,        // INV_CONSTANT
        0x8003 => const_a,                         // CONSTANT_ALPHA
        0x8004 => 1.0 - const_a,                   // INV_CONSTANT_ALPHA
        _ => 0.5,
    }
}

#[allow(dead_code, reason = "Bochs geforce.cc `blend_equation`, ported ahead of its caller")]
fn blend_equation(equation: u16, src: f32, src_factor: f32, dst: f32, dst_factor: f32) -> f32 {
    match equation {
        0x0002 | 0x800a => src * src_factor - dst * dst_factor, // SUBTRACT
        0x0003 | 0x800b => dst * dst_factor - src * src_factor, // REV_SUBTRACT
        0x0004 | 0x8007 => src.min(dst),                        // MIN
        0x0005 | 0x8008 => src.max(dst),                        // MAX
        _ => src * src_factor + dst * dst_factor,               // ADD (default)
    }
}

#[allow(dead_code, reason = "Bochs geforce.cc `swizzle`, ported ahead of its caller")]
fn swizzle_addr(x: u32, y: u32, width: u32, height: u32) -> u32 {
    let mut xleft = true;
    let mut yleft = height != 1;
    let mut xbit = 1u32;
    let mut ybit = 1u32;
    let mut rbit = 1u32;
    let mut r = 0u32;
    loop {
        if xleft {
            if (x & xbit) != 0 {
                r |= rbit;
            }
            rbit <<= 1;
            xbit <<= 1;
            xleft = xbit < width;
        }
        if yleft {
            if (y & ybit) != 0 {
                r |= rbit;
            }
            rbit <<= 1;
            ybit <<= 1;
            yleft = ybit < height;
        }
        if !xleft && !yleft {
            break;
        }
    }
    r
}

#[allow(dead_code, reason = "Bochs geforce.cc `unpack_attribute`, ported ahead of its caller")]
fn unpack_attribute(value: u32, d3d: bool, comp: &mut [f32; 4]) {
    if d3d {
        comp[0] = ((value >> 16) & 0xff) as f32 / 255.0;
        comp[1] = ((value >> 8) & 0xff) as f32 / 255.0;
        comp[2] = (value & 0xff) as f32 / 255.0;
        comp[3] = ((value >> 24) & 0xff) as f32 / 255.0;
    } else {
        for i in 0..4 {
            comp[i] = ((value >> (i * 8)) & 0xff) as f32 / 255.0;
        }
    }
}

/// Register combiner variable extraction
#[allow(dead_code, reason = "Bochs geforce.cc `rc_get_var`, ported ahead of its caller")]
fn rc_get_var(cw: u32, shift: u32, regs: &[[f32; 4]; 16], civ: u32) -> f32 {
    let x = cw >> shift;
    let reg = (x & 0xf) as usize;
    let pir = (x >> 4) & 1;
    let map = (x >> 5) & 7;
    let cir = if pir != 0 { 3 } else { civ } as usize;
    let value = regs[reg][cir];
    match map {
        0 => value.max(0.0),              // UNSIGNED_IDENTITY
        1 => 1.0 - value.clamp(0.0, 1.0), // UNSIGNED_INVERT
        2 => 2.0 * value.max(0.0) - 1.0,  // EXPAND_NORMAL
        3 => -2.0 * value.max(0.0) + 1.0, // EXPAND_NEGATE
        4 => value.max(0.0) - 0.5,        // HALF_BIAS_NORMAL
        5 => -value.max(0.0) + 0.5,       // HALF_BIAS_NEGATE
        6 => value,                       // SIGNED_IDENTITY
        7 => -value,                      // SIGNED_NEGATE
        _ => value,
    }
}

#[allow(
    dead_code,
    reason = "Bochs geforce.cc `texture_process_format`, ported ahead of its caller"
)]
fn texture_process_format(tex: &mut GfTexture) {
    tex.linear = false;
    tex.unnormalized = false;
    tex.compressed = false;
    tex.dxt_alpha_data = false;
    tex.dxt_alpha_explicit = false;
    if (tex.format & 0x80) != 0 {
        if (tex.format & 0x20) != 0 {
            tex.linear = true;
        }
        if (tex.format & 0x40) != 0 {
            tex.unnormalized = true;
        }
        tex.format &= 0x9f;
    } else if tex.format == 0x12 || tex.format == 0x1b || tex.format == 0x1e {
        tex.linear = true;
        tex.unnormalized = true;
    }
    match tex.format {
        0x0c | 0x0e | 0x0f | 0x86 | 0x87 | 0x88 => {
            tex.compressed = true;
            tex.dxt_alpha_data = tex.format != 0x0c && tex.format != 0x86;
            tex.dxt_alpha_explicit = tex.format == 0x0e || tex.format == 0x87;
            tex.color_bytes = if tex.dxt_alpha_data { 16 } else { 8 };
        }
        0x02 | 0x03 | 0x04 | 0x05 | 0x27 | 0x28 | 0x82 | 0x83 | 0x84 | 0x8b | 0x8f => {
            tex.color_bytes = 2;
        }
        0x06 | 0x07 | 0x12 | 0x1e | 0x85 => {
            tex.color_bytes = 4;
        }
        _ => {
            tex.color_bytes = 1;
        }
    }
}

#[allow(dead_code, reason = "Bochs geforce.cc `texture_update_size`, ported ahead of its caller")]
fn texture_update_size(tex: &mut GfTexture, cls: u32) {
    if tex.linear || cls >= 0x4097 {
        tex.size[0] = tex.image_rect >> 16;
        tex.size[1] = tex.image_rect & 0x0000ffff;
    } else {
        tex.size[0] = 1 << tex.base_size[0];
        tex.size[1] = 1 << tex.base_size[1];
    }
    let mut lw = tex.size[0];
    let mut lh = tex.size[1];
    tex.face_bytes = 0;
    for _ in 0..tex.levels {
        let mut level_bytes = lw * lh * tex.color_bytes;
        if tex.compressed {
            level_bytes /= 16;
        }
        tex.face_bytes += level_bytes;
        lw = (lw / 2).max(1);
        lh = (lh / 2).max(1);
    }
    // Each cubemap face starts on a 128-byte boundary, so the per-face stride
    // is the mip-chain size rounded up — without this the second and later
    // faces are sampled from the wrong offset. Bochs geforce.cc
    // texture_update_size.
    tex.face_bytes = align_up(tex.face_bytes, 128);
}

// ---------------------------------------------------------------------------
// Main GPU struct
// ---------------------------------------------------------------------------

/// GeForce GPU emulation state.
///
/// Contains all register banks, VRAM, channel state, and display state.
pub struct BxGeForceC {
    // CRTC registers
    pub crtc: CrtcRegs,

    // PMC state
    pub mc_soft_intr: bool,
    pub mc_intr_en: u32,
    pub mc_enable: u32,

    // PBUS state
    pub bus_intr: u32,
    pub bus_intr_en: u32,

    // PFIFO state
    pub fifo_wait: bool,
    pub fifo_wait_soft: bool,
    pub fifo_wait_notify: bool,
    pub fifo_wait_flip: bool,
    pub fifo_wait_acquire: bool,
    pub fifo_intr: u32,
    pub fifo_intr_en: u32,
    pub fifo_ramht: u32,
    pub fifo_ramfc: u32,
    pub fifo_ramro: u32,
    pub fifo_mode: u32,
    pub fifo_cache1_push0: u32,
    pub fifo_cache1_push1: u32,
    pub fifo_cache1_put: u32,
    pub fifo_cache1_dma_push: u32,
    pub fifo_cache1_dma_instance: u32,
    pub fifo_cache1_dma_put: u32,
    pub fifo_cache1_dma_get: u32,
    pub fifo_cache1_ref_cnt: u32,
    pub fifo_cache1_pull0: u32,
    pub fifo_cache1_semaphore: u32,
    pub fifo_cache1_get: u32,
    pub fifo_grctx_instance: u32,
    pub fifo_cache1_method: [u32; GEFORCE_CACHE1_SIZE],
    pub fifo_cache1_data: [u32; GEFORCE_CACHE1_SIZE],

    // PRMA state
    pub rma_addr: u32,

    // PTIMER state
    pub timer_intr: u32,
    pub timer_intr_en: u32,
    pub timer_num: u32,
    pub timer_den: u32,
    pub timer_inittime1: u64,
    pub timer_inittime2: u64,
    pub timer_alarm: u32,

    // PFB / straps
    pub straps0_primary: u32,
    pub straps0_primary_original: u32,

    // PGRAPH state
    pub graph_intr: u32,
    pub graph_nsource: u32,
    pub graph_intr_en: u32,
    pub graph_ctx_switch1: u32,
    pub graph_ctx_switch2: u32,
    pub graph_ctx_switch4: u32,
    pub graph_ctxctl_cur: u32,
    pub graph_status: u32,
    pub graph_trapped_addr: u32,
    pub graph_trapped_data: u32,
    pub graph_flip_read: u32,
    pub graph_flip_write: u32,
    pub graph_flip_modulo: u32,
    pub graph_notify: u32,
    pub graph_fifo: u32,
    pub graph_bpixel: u32,
    pub graph_channel_ctx_table: u32,
    pub graph_offset0: u32,
    pub graph_pitch0: u32,

    // PCRTC state
    pub crtc_intr: u32,
    pub crtc_intr_en: u32,
    pub crtc_start: u32,
    pub crtc_config: u32,
    pub crtc_raster_pos: u32,
    pub crtc_cursor_offset: u32,
    pub crtc_cursor_config: u32,
    pub crtc_gpio_ext: u32,

    // PRAMDAC state
    pub ramdac_cu_start_pos: u32,
    pub ramdac_vpll: u32,
    pub ramdac_vpll_b: u32,
    pub ramdac_pll_select: u32,
    pub ramdac_general_control: u32,

    // ROP handlers (256 entries)
    pub rop_handler: [RopHandler; 256],
    pub rop_flags: [u8; 256],

    // Channels
    pub chs: Vec<GfChannel>,

    // Unknown registers (temporary storage for unhandled addresses)
    pub unk_regs: Vec<u32>,

    // VGA/SVGA display state
    pub svga_unlock_special: bool,
    pub svga_needs_update_tile: bool,
    pub svga_needs_update_dispentire: bool,
    pub svga_needs_update_mode: bool,
    pub svga_double_width: bool,
    pub svga_xres: u32,
    pub svga_yres: u32,
    pub svga_pitch: u32,
    pub svga_bpp: u32,
    pub svga_dispbpp: u32,

    // Card identity
    pub card_type: u32,
    pub memsize: u32,
    pub memsize_mask: u32,
    pub bar2_size: u32,
    pub ramin_flip: u32,
    pub class_mask: u32,

    // VRAM
    pub memory: Vec<u8>,

    // Display pointers
    pub disp_offset: u32,
    pub disp_end_offset: u32,
    pub bank_base: [u32; 2],

    // Hardware cursor
    pub hw_cursor: HwCursor,

    /// The monitor's DDC channel, bit-banged through CRTC 0x3F and read back
    /// through 0x3E — Bochs geforce.h `ddc`.
    ddc: BxDdcC,

    // PCI configuration space (256 bytes)
    pub pci_conf: [u8; 256],

    // PCI ROM (optional)
    pub pci_rom: Vec<u8>,

    // Elapsed time tracking (nanoseconds)
    pub time_nsec: u64,
}

impl BxGeForceC {
    /// Create a new GeForce GPU instance with the specified model.
    pub fn new(model: GeForceModel) -> Self {
        let card_type = model.card_type();
        let memsize: u32 = match card_type {
            0x15 => 64 * 1024 * 1024,
            0x20 => 64 * 1024 * 1024,
            0x35 => 128 * 1024 * 1024,
            _ => 256 * 1024 * 1024, // 6800
        };

        let bar2_size: u32 = match card_type {
            0x15 => 0, // no BAR2
            0x20 => 0x0008_0000,
            _ => 0x0100_0000,
        };

        let straps0_primary_original = if card_type <= 0x20 {
            0x7FF8_6C6B | 0x0000_0180
        } else {
            0x7FF8_6C4B | 0x0000_0180
        };

        let ramin_flip = memsize - 64;
        let memsize_mask = memsize - 1;
        let class_mask = if card_type < 0x40 {
            0x0000_0FFF
        } else {
            0x0000_FFFF
        };

        let mut rop_handler = [rop_nop as RopHandler; 256];
        let mut rop_flags = [BX_ROP_PATTERN; 256];
        // Initialize ROP table matching C++ bitblt_init()
        Self::init_rop_table(&mut rop_handler, &mut rop_flags);

        let mut chs = Vec::with_capacity(GEFORCE_CHANNEL_COUNT);
        for _ in 0..GEFORCE_CHANNEL_COUNT {
            chs.push(GfChannel::new());
        }

        let memory = vec![0u8; memsize as usize];
        let unk_regs = vec![0u32; 4 * 1024 * 1024];

        tracing::debug!(
            "{} initialized, VRAM {}MB",
            model.name(),
            memsize / (1024 * 1024)
        );

        Self {
            crtc: CrtcRegs::default(),
            mc_soft_intr: false,
            mc_intr_en: 0,
            mc_enable: 0,
            bus_intr: 0,
            bus_intr_en: 0,
            fifo_wait: false,
            fifo_wait_soft: false,
            fifo_wait_notify: false,
            fifo_wait_flip: false,
            fifo_wait_acquire: false,
            fifo_intr: 0,
            fifo_intr_en: 0,
            fifo_ramht: 0,
            fifo_ramfc: 0,
            fifo_ramro: 0,
            fifo_mode: 0,
            fifo_cache1_push0: 0,
            fifo_cache1_push1: 0,
            fifo_cache1_put: 0,
            fifo_cache1_dma_push: 0,
            fifo_cache1_dma_instance: 0,
            fifo_cache1_dma_put: 0,
            fifo_cache1_dma_get: 0,
            fifo_cache1_ref_cnt: 0,
            fifo_cache1_pull0: 0,
            fifo_cache1_semaphore: 0,
            fifo_cache1_get: 0,
            fifo_grctx_instance: 0,
            fifo_cache1_method: [0; GEFORCE_CACHE1_SIZE],
            fifo_cache1_data: [0; GEFORCE_CACHE1_SIZE],
            rma_addr: 0,
            timer_intr: 0,
            timer_intr_en: 0,
            timer_num: 0,
            timer_den: 0,
            timer_inittime1: 0,
            timer_inittime2: 0,
            timer_alarm: 0,
            straps0_primary: straps0_primary_original,
            straps0_primary_original,
            graph_intr: 0,
            graph_nsource: 0,
            graph_intr_en: 0,
            graph_ctx_switch1: 0,
            graph_ctx_switch2: 0,
            graph_ctx_switch4: 0,
            graph_ctxctl_cur: 0,
            graph_status: 0,
            graph_trapped_addr: 0,
            graph_trapped_data: 0,
            graph_flip_read: 0,
            graph_flip_write: 0,
            graph_flip_modulo: 0,
            graph_notify: 0,
            graph_fifo: 0,
            graph_bpixel: 0,
            graph_channel_ctx_table: 0,
            graph_offset0: 0,
            graph_pitch0: 0,
            crtc_intr: 0,
            crtc_intr_en: 0,
            crtc_start: 0,
            crtc_config: 0,
            crtc_raster_pos: 0,
            crtc_cursor_offset: 0,
            crtc_cursor_config: 0,
            crtc_gpio_ext: 0,
            ramdac_cu_start_pos: 0,
            ramdac_vpll: 0,
            ramdac_vpll_b: 0,
            ramdac_pll_select: 0,
            ramdac_general_control: 0,
            rop_handler,
            rop_flags,
            chs,
            unk_regs,
            svga_unlock_special: false,
            svga_needs_update_tile: true,
            svga_needs_update_dispentire: true,
            svga_needs_update_mode: false,
            svga_double_width: false,
            svga_xres: 640,
            svga_yres: 480,
            svga_pitch: 640,
            svga_bpp: 8,
            svga_dispbpp: 0,
            card_type,
            memsize,
            memsize_mask,
            bar2_size,
            ramin_flip,
            class_mask,
            memory,
            disp_offset: 0,
            disp_end_offset: 0,
            bank_base: [0; 2],
            hw_cursor: HwCursor {
                size: 32,
                ..HwCursor::default()
            },
            ddc: BxDdcC::new(),
            pci_conf: [0u8; 256],
            pci_rom: Vec::new(),
            time_nsec: 0,
        }
    }

    fn init_rop_table(handlers: &mut [RopHandler; 256], flags: &mut [u8; 256]) {
        // Default: all NOPs with pattern flag
        for i in 0..256 {
            handlers[i] = rop_nop;
            flags[i] = BX_ROP_PATTERN;
        }
        // Binary ROPs without pattern
        handlers[0x00] = rop_0;
        flags[0x00] = 0;
        handlers[0x11] = rop_notsrc_and_notdst;
        flags[0x11] = 0;
        handlers[0x22] = rop_notsrc_and_dst;
        flags[0x22] = 0;
        handlers[0x33] = rop_notsrc;
        flags[0x33] = 0;
        handlers[0x44] = rop_src_and_notdst;
        flags[0x44] = 0;
        handlers[0x55] = rop_notdst;
        flags[0x55] = 0;
        handlers[0x66] = rop_src_xor_dst;
        flags[0x66] = 0;
        handlers[0x77] = rop_notsrc_or_notdst;
        flags[0x77] = 0;
        handlers[0x88] = rop_src_and_dst;
        flags[0x88] = 0;
        handlers[0x99] = rop_src_notxor_dst;
        flags[0x99] = 0;
        handlers[0xaa] = rop_nop;
        flags[0xaa] = 0;
        handlers[0xbb] = rop_notsrc_or_dst;
        flags[0xbb] = 0;
        handlers[0xcc] = rop_src;
        flags[0xcc] = 0;
        handlers[0xdd] = rop_src_and_notdst;
        flags[0xdd] = 0;
        handlers[0xee] = rop_src_or_dst;
        flags[0xee] = 0;
        handlers[0xff] = rop_1;
        flags[0xff] = 0;
        // Pattern ROPs
        handlers[0x05] = rop_notsrc_and_notdst;
        handlers[0x0a] = rop_notsrc_and_dst;
        handlers[0x0f] = rop_notsrc;
        handlers[0x50] = rop_src_and_notdst;
        flags[0x50] = BX_ROP_PATTERN; // already default
        handlers[0x5a] = rop_src_xor_dst;
        handlers[0x5f] = rop_notsrc_or_notdst;
        handlers[0xad] = rop_src_and_dst;
        handlers[0xaf] = rop_notsrc_or_dst;
        handlers[0xf0] = rop_src;
        handlers[0xf5] = rop_src_or_notdst;
        handlers[0xfa] = rop_src_or_dst;
    }

    /// Reset the NV chip — Bochs geforce.cc `bx_geforce_c::reset` after its
    /// `bx_vgacore_c::reset` half: `svga_init_members`, `ddc.init()`, and PCI
    /// config 0x50 cleared to disable ROM shadowing.
    pub fn reset(&mut self) {
        self.crtc = CrtcRegs::default();
        self.mc_soft_intr = false;
        self.mc_intr_en = 0;
        self.mc_enable = 0;
        self.bus_intr = 0;
        self.bus_intr_en = 0;
        self.fifo_wait = false;
        self.fifo_wait_soft = false;
        self.fifo_wait_notify = false;
        self.fifo_wait_flip = false;
        self.fifo_wait_acquire = false;
        self.fifo_intr = 0;
        self.fifo_intr_en = 0;
        self.fifo_ramht = 0;
        self.fifo_ramfc = 0;
        self.fifo_ramro = 0;
        self.fifo_mode = 0;
        self.fifo_cache1_push0 = 0;
        self.fifo_cache1_push1 = 0;
        self.fifo_cache1_put = 0;
        self.fifo_cache1_dma_push = 0;
        self.fifo_cache1_dma_instance = 0;
        self.fifo_cache1_dma_put = 0;
        self.fifo_cache1_dma_get = 0;
        self.fifo_cache1_ref_cnt = 0;
        self.fifo_cache1_pull0 = 0;
        self.fifo_cache1_semaphore = 0;
        self.fifo_cache1_get = 0;
        self.fifo_grctx_instance = 0;
        self.fifo_cache1_method = [0; GEFORCE_CACHE1_SIZE];
        self.fifo_cache1_data = [0; GEFORCE_CACHE1_SIZE];
        self.rma_addr = 0;
        self.timer_intr = 0;
        self.timer_intr_en = 0;
        self.timer_num = 0;
        self.timer_den = 0;
        self.timer_inittime1 = 0;
        self.timer_inittime2 = 0;
        self.timer_alarm = 0;
        self.graph_intr = 0;
        self.graph_nsource = 0;
        self.graph_intr_en = 0;
        self.graph_ctx_switch1 = 0;
        self.graph_ctx_switch2 = 0;
        self.graph_ctx_switch4 = 0;
        self.graph_ctxctl_cur = 0;
        self.graph_status = 0;
        self.graph_trapped_addr = 0;
        self.graph_trapped_data = 0;
        self.graph_flip_read = 0;
        self.graph_flip_write = 0;
        self.graph_flip_modulo = 0;
        self.graph_notify = 0;
        self.graph_fifo = 0;
        self.graph_bpixel = 0;
        self.graph_channel_ctx_table = 0;
        self.graph_offset0 = 0;
        self.graph_pitch0 = 0;
        self.crtc_intr = 0;
        self.crtc_intr_en = 0;
        self.crtc_start = 0;
        self.crtc_config = 0;
        self.crtc_raster_pos = 0;
        self.crtc_cursor_offset = 0;
        self.crtc_cursor_config = 0;
        self.crtc_gpio_ext = 0;
        self.ramdac_cu_start_pos = 0;
        self.ramdac_vpll = 0;
        self.ramdac_vpll_b = 0;
        self.ramdac_pll_select = 0;
        self.ramdac_general_control = 0;

        for ch in self.chs.iter_mut() {
            *ch = GfChannel::new();
        }
        for r in self.unk_regs.iter_mut() {
            *r = 0;
        }

        self.svga_unlock_special = false;
        self.svga_needs_update_tile = true;
        self.svga_needs_update_dispentire = true;
        self.svga_needs_update_mode = false;
        self.svga_double_width = false;
        self.svga_xres = 640;
        self.svga_yres = 480;
        self.svga_bpp = 8;
        self.svga_pitch = 640;
        self.bank_base = [0; 2];
        self.hw_cursor = HwCursor {
            size: 32,
            ..HwCursor::default()
        };
        self.disp_offset = 0;
        self.disp_end_offset = 0;
        self.memory.fill(0);
        self.straps0_primary = self.straps0_primary_original;

        self.ddc = BxDdcC::new();
        self.pci_conf[0x50] = 0x00;
    }

    // -----------------------------------------------------------------------
    // VRAM access
    // -----------------------------------------------------------------------

    /// `N` bytes of video memory from `address`, in memory order.
    ///
    /// A byte past the end of VRAM reads as zero. Bochs geforce.cc
    /// `vram_read8`..`vram_read64` index `s.memory` unbounded, and a guest
    /// reaches the last bytes of VRAM with a multi-byte access that straddles
    /// the end — the RMA window checks only the first byte against `memsize`.
    fn vram_load<const N: usize>(&self, address: u32) -> [u8; N] {
        let start = address as usize;
        core::array::from_fn(|index| {
            start
                .checked_add(index)
                .and_then(|at| self.memory.get(at))
                .copied()
                .unwrap_or(0)
        })
    }

    /// Store `bytes` into video memory from `address`, dropping any byte past
    /// the end of VRAM — the write half of [`Self::vram_load`].
    fn vram_store(&mut self, address: u32, bytes: &[u8]) {
        let start = address as usize;
        for (index, &byte) in bytes.iter().enumerate() {
            if let Some(slot) = start
                .checked_add(index)
                .and_then(|at| self.memory.get_mut(at))
            {
                *slot = byte;
            }
        }
    }

    pub fn vram_read8(&self, address: u32) -> u8 {
        u8::from_le_bytes(self.vram_load(address))
    }

    pub fn vram_read16(&self, address: u32) -> u16 {
        u16::from_le_bytes(self.vram_load(address))
    }

    pub fn vram_read32(&self, address: u32) -> u32 {
        u32::from_le_bytes(self.vram_load(address))
    }

    pub fn vram_read64(&self, address: u32) -> u64 {
        u64::from_le_bytes(self.vram_load(address))
    }

    pub fn vram_write8(&mut self, address: u32, value: u8) {
        self.vram_store(address, &[value]);
    }

    pub fn vram_write16(&mut self, address: u32, value: u16) {
        self.vram_store(address, &value.to_le_bytes());
    }

    pub fn vram_write32(&mut self, address: u32, value: u32) {
        self.vram_store(address, &value.to_le_bytes());
    }

    pub fn vram_write64(&mut self, address: u32, value: u64) {
        self.vram_store(address, &value.to_le_bytes());
    }

    // -----------------------------------------------------------------------
    // RAMIN access (VRAM with flip offset)
    // -----------------------------------------------------------------------

    pub fn ramin_read8(&self, address: u32) -> u8 {
        self.vram_read8(address ^ self.ramin_flip)
    }

    pub fn ramin_read16(&self, address: u32) -> u16 {
        self.vram_read16(address ^ self.ramin_flip)
    }

    pub fn ramin_read32(&self, address: u32) -> u32 {
        self.vram_read32(address ^ self.ramin_flip)
    }

    pub fn ramin_write8(&mut self, address: u32, value: u8) {
        self.vram_write8(address ^ self.ramin_flip, value);
    }

    pub fn ramin_write32(&mut self, address: u32, value: u32) {
        self.vram_write32(address ^ self.ramin_flip, value);
    }

    // -----------------------------------------------------------------------
    // DMA address translation
    // -----------------------------------------------------------------------

    fn dma_pt_lookup(&self, object: u32, address: u32) -> u32 {
        let address_adj = address.wrapping_add(self.ramin_read32(object) >> 20);
        let page_offset = address_adj & 0xFFF;
        let page_index = address_adj >> 12;
        let page = self.ramin_read32(object + 8 + page_index * 4) & 0xFFFF_F000;
        page | page_offset
    }

    fn dma_lin_lookup(&self, object: u32, address: u32) -> u32 {
        let adjust = self.ramin_read32(object) >> 20;
        let base = self.ramin_read32(object + 8) & 0xFFFF_F000;
        base.wrapping_add(adjust).wrapping_add(address)
    }

    fn dma_abs_addr(&self, object: u32, address: u32) -> (u32, bool) {
        let flags = self.ramin_read32(object);
        let addr = if flags & 0x0000_2000 != 0 {
            self.dma_lin_lookup(object, address)
        } else {
            self.dma_pt_lookup(object, address)
        };
        let is_physical = flags & 0x0002_0000 != 0;
        (addr, is_physical)
    }

    pub fn dma_read8(&self, object: u32, address: u32) -> u8 {
        let (addr, is_phys) = self.dma_abs_addr(object, address);
        if is_phys {
            // Physical memory read - stub: return 0
            tracing::debug!("DMA physical read8 at {:#010x}", addr);
            0
        } else {
            self.vram_read8(addr & self.memsize_mask)
        }
    }

    pub fn dma_read16(&self, object: u32, address: u32) -> u16 {
        let (addr, is_phys) = self.dma_abs_addr(object, address);
        if is_phys {
            tracing::debug!("DMA physical read16 at {:#010x}", addr);
            0
        } else {
            self.vram_read16(addr & self.memsize_mask)
        }
    }

    pub fn dma_read32(&self, object: u32, address: u32) -> u32 {
        let (addr, is_phys) = self.dma_abs_addr(object, address);
        if is_phys {
            tracing::debug!("DMA physical read32 at {:#010x}", addr);
            0
        } else {
            self.vram_read32(addr & self.memsize_mask)
        }
    }

    pub fn dma_read64(&self, object: u32, address: u32) -> u64 {
        let (addr, is_phys) = self.dma_abs_addr(object, address);
        if is_phys {
            tracing::debug!("DMA physical read64 at {:#010x}", addr);
            0
        } else {
            self.vram_read64(addr & self.memsize_mask)
        }
    }

    pub fn dma_write8(&mut self, object: u32, address: u32, value: u8) {
        let (addr, is_phys) = self.dma_abs_addr(object, address);
        if is_phys {
            tracing::debug!("DMA physical write8 at {:#010x}", addr);
        } else {
            self.vram_write8(addr & self.memsize_mask, value);
        }
    }

    pub fn dma_write16(&mut self, object: u32, address: u32, value: u16) {
        let (addr, is_phys) = self.dma_abs_addr(object, address);
        if is_phys {
            tracing::debug!("DMA physical write16 at {:#010x}", addr);
        } else {
            self.vram_write16(addr & self.memsize_mask, value);
        }
    }

    pub fn dma_write32(&mut self, object: u32, address: u32, value: u32) {
        let (addr, is_phys) = self.dma_abs_addr(object, address);
        if is_phys {
            tracing::debug!("DMA physical write32 at {:#010x}", addr);
        } else {
            self.vram_write32(addr & self.memsize_mask, value);
        }
    }

    pub fn dma_write64(&mut self, object: u32, address: u32, value: u64) {
        let (addr, is_phys) = self.dma_abs_addr(object, address);
        if is_phys {
            tracing::debug!("DMA physical write64 at {:#010x}", addr);
        } else {
            self.vram_write64(addr & self.memsize_mask, value);
        }
    }

    pub fn dma_copy(
        &mut self,
        dst_obj: u32,
        dst_addr: u32,
        src_obj: u32,
        src_addr: u32,
        byte_count: u32,
    ) {
        // Simplified: copy via VRAM only (no physical memory support)
        let mut src_off = src_addr;
        let mut dst_off = dst_addr;
        let mut remaining = byte_count;
        while remaining > 0 {
            let chunk = remaining.min(4096);
            let mut buf = vec![0u8; chunk as usize];
            for i in 0..chunk {
                buf[i as usize] = self.dma_read8(src_obj, src_off + i);
            }
            for i in 0..chunk {
                self.dma_write8(dst_obj, dst_off + i, buf[i as usize]);
            }
            src_off += chunk;
            dst_off += chunk;
            remaining -= chunk;
        }
    }

    // -----------------------------------------------------------------------
    // RAMFC / RAMHT
    // -----------------------------------------------------------------------

    fn ramfc_address(&self, chid: u32, offset: u32) -> u32 {
        let ramfc = if self.card_type < 0x40 {
            (self.fifo_ramfc & 0xFFF) << 8
        } else {
            (self.fifo_ramfc & 0xFFF) << 16
        };
        let ch_size = if self.card_type < 0x20 {
            0x20
        } else if self.card_type < 0x40 {
            0x40
        } else {
            0x80
        };
        ramfc + chid * ch_size + offset
    }

    fn ramfc_write32(&mut self, chid: u32, offset: u32, value: u32) {
        let addr = self.ramfc_address(chid, offset);
        self.ramin_write32(addr, value);
    }

    fn ramfc_read32(&self, chid: u32, offset: u32) -> u32 {
        let addr = self.ramfc_address(chid, offset);
        self.ramin_read32(addr)
    }

    fn ramht_lookup(&self, handle: u32, chid: u32) -> (u32, u8) {
        let ramht_addr = (self.fifo_ramht & 0xFFF) << 8;
        let ramht_bits = ((self.fifo_ramht >> 16) & 0xFF) + 9;
        let ramht_size = (1u32 << ramht_bits) << 3;

        let mut hash = 0u32;
        let mut x = handle;
        while x != 0 {
            hash ^= x & ((1 << ramht_bits) - 1);
            x >>= ramht_bits;
        }
        hash ^= (chid & 0xF) << (ramht_bits - 4);
        hash <<= 3;

        let mut it = hash;
        loop {
            if self.ramin_read32(ramht_addr + it) == handle {
                let context = self.ramin_read32(ramht_addr + it + 4);
                let ctx_chid = if self.card_type < 0x40 {
                    (context >> 24) & 0x1F
                } else {
                    (context >> 23) & 0x1F
                };
                if chid == ctx_chid {
                    let object = if self.card_type < 0x40 {
                        (context & 0xFFFF) << 4
                    } else {
                        (context & 0xFFFFF) << 4
                    };
                    let engine = if self.card_type < 0x40 {
                        ((context >> 16) & 0xFF) as u8
                    } else {
                        ((context >> 20) & 0x7) as u8
                    };
                    return (object, engine);
                }
            }
            it += 8;
            if it >= ramht_size {
                it = 0;
            }
            if it == hash {
                break;
            }
        }

        tracing::error!("ramht_lookup failed for {:#010x}", handle);
        (0, 0)
    }

    fn get_current_time(&self) -> u64 {
        (self
            .timer_inittime1
            .wrapping_add(self.time_nsec.wrapping_sub(self.timer_inittime2)))
            & !0x1Fu64
    }

    // -----------------------------------------------------------------------
    // IRQ management
    // -----------------------------------------------------------------------

    fn get_mc_intr(&self) -> u32 {
        let mut value = 0u32;
        if self.bus_intr & self.bus_intr_en != 0 {
            value |= 0x1000_0000;
        }
        if self.fifo_intr & self.fifo_intr_en != 0 {
            value |= 0x0000_0100;
        }
        if self.graph_intr & self.graph_intr_en != 0 {
            value |= 0x0000_1000;
        }
        if self.crtc_intr & self.crtc_intr_en != 0 {
            value |= 0x0100_0000;
        }
        value
    }

    fn update_irq_level(&self) {
        let level = (self.get_mc_intr() != 0 && self.mc_intr_en & 1 != 0)
            || (self.mc_soft_intr && self.mc_intr_en & 2 != 0);
        // In a full integration, this would call DEV_pci_set_irq
        if level {
            tracing::debug!("GeForce IRQ raised");
        }
    }

    fn update_fifo_wait(&mut self) {
        self.fifo_wait = self.fifo_wait_soft
            || self.fifo_wait_notify
            || self.fifo_wait_flip
            || self.fifo_wait_acquire;
    }

    // -----------------------------------------------------------------------
    // Pixel operations
    // -----------------------------------------------------------------------

    #[allow(
        dead_code,
        reason = "Bochs geforce.cc `bx_geforce_c::get_pixel`, ported ahead of its caller"
    )]
    fn get_pixel(&self, obj: u32, ofs: u32, x: u32, cb: u32) -> u32 {
        match cb {
            1 => self.dma_read8(obj, ofs + x) as u32,
            2 => self.dma_read16(obj, ofs + x * 2) as u32,
            _ => self.dma_read32(obj, ofs + x * 4),
        }
    }

    #[allow(
        dead_code,
        reason = "Bochs geforce.cc `bx_geforce_c::put_pixel`, ported ahead of its caller"
    )]
    fn put_pixel(
        &mut self,
        ch_s2d_img_dst: u32,
        ch_s2d_color_bytes: u32,
        ch_s2d_color_fmt: u32,
        ofs: u32,
        x: u32,
        value: u32,
    ) {
        match ch_s2d_color_bytes {
            1 => self.dma_write8(ch_s2d_img_dst, ofs + x, value as u8),
            2 => self.dma_write16(ch_s2d_img_dst, ofs + x * 2, value as u16),
            _ => {
                let v = if ch_s2d_color_fmt == 6 {
                    value & 0x00FF_FFFF
                } else {
                    value
                };
                self.dma_write32(ch_s2d_img_dst, ofs + x * 4, v);
            }
        }
    }

    #[allow(
        dead_code,
        reason = "Bochs geforce.cc `bx_geforce_c::put_pixel_swzs`, ported ahead of its caller"
    )]
    fn put_pixel_swzs(
        &mut self,
        ch_swzs_img_obj: u32,
        ch_swzs_color_bytes: u32,
        ofs: u32,
        value: u32,
    ) {
        match ch_swzs_color_bytes {
            1 => self.dma_write8(ch_swzs_img_obj, ofs, value as u8),
            2 => self.dma_write16(ch_swzs_img_obj, ofs, value as u16),
            _ => self.dma_write32(ch_swzs_img_obj, ofs, value),
        }
    }

    #[allow(
        dead_code,
        reason = "Bochs geforce.cc `bx_geforce_c::pixel_operation`, ported ahead of its caller"
    )]
    fn pixel_operation(
        &self,
        ch: &GfChannel,
        op: u32,
        dstcolor: &mut u32,
        srccolor: &u32,
        cb: u32,
        _px: u32,
        _py: u32,
    ) {
        if op == 1 {
            // ROP operation
            let rop = ch.rop;
            if self.rop_flags[rop as usize] != 0 {
                let i = (_py % 8 * 8 + _px % 8) as usize;
                let patt_color = if ch.patt_type_color {
                    ch.patt_data_color[i]
                } else if ch.patt_data_mono[i] {
                    ch.patt_fg_color
                } else {
                    ch.patt_bg_color
                };
                let mut dst_bytes = dstcolor.to_le_bytes();
                let src_bytes = srccolor.to_le_bytes();
                let pat_bytes = patt_color.to_le_bytes();
                bx_ternary_rop(
                    rop,
                    &mut dst_bytes[..cb as usize],
                    &src_bytes[..cb as usize],
                    &pat_bytes[..cb as usize],
                    cb,
                );
                *dstcolor = u32::from_le_bytes(dst_bytes);
            } else {
                let mut dst_bytes = dstcolor.to_le_bytes();
                let src_bytes = srccolor.to_le_bytes();
                (self.rop_handler[rop as usize])(
                    &mut dst_bytes[..cb as usize],
                    &src_bytes[..cb as usize],
                    0,
                    0,
                    cb,
                    1,
                );
                *dstcolor = u32::from_le_bytes(dst_bytes);
            }
        } else if op == 5 {
            // Alpha blending operation
            if cb == 4 {
                if *srccolor != 0 {
                    let sb = (*srccolor & 0xFF) as u8;
                    let sg = ((*srccolor >> 8) & 0xFF) as u8;
                    let sr = ((*srccolor >> 16) & 0xFF) as u8;
                    let sa = ((*srccolor >> 24) & 0xFF) as u8;
                    let db = (*dstcolor & 0xFF) as u8;
                    let dg = ((*dstcolor >> 8) & 0xFF) as u8;
                    let dr = ((*dstcolor >> 16) & 0xFF) as u8;
                    let da = ((*dstcolor >> 24) & 0xFF) as u8;
                    let isa = 0xFFu16 - sa as u16;
                    let b = alpha_wrap((db as i32 * isa as i32 / 0xFF) + sb as i32);
                    let g = alpha_wrap((dg as i32 * isa as i32 / 0xFF) + sg as i32);
                    let r = alpha_wrap((dr as i32 * isa as i32 / 0xFF) + sr as i32);
                    let a = alpha_wrap((da as i32 * isa as i32 / 0xFF) + sa as i32);
                    *dstcolor =
                        (b as u32) | ((g as u32) << 8) | ((r as u32) << 16) | ((a as u32) << 24);
                }
            } else {
                *dstcolor = *srccolor;
            }
        } else {
            *dstcolor = *srccolor;
        }
    }

    // -----------------------------------------------------------------------
    // Register read (MMIO dispatch)
    // -----------------------------------------------------------------------

    pub fn register_read32(&self, address: u32) -> u32 {
        match address {
            0x0 => {
                if self.card_type == 0x20 {
                    0x0202_00A5
                } else {
                    self.card_type << 20
                }
            }
            0x100 => {
                let mut v = self.get_mc_intr();
                if self.mc_soft_intr {
                    v |= 0x8000_0000;
                }
                v
            }
            0x140 => self.mc_intr_en,
            0x200 => self.mc_enable,
            0x1100 => self.bus_intr,
            0x1140 => self.bus_intr_en,
            a if a >= 0x1800 && a < 0x1900 => {
                // PCI config space mirrored into BAR0, as Bochs geforce.cc
                // `register_read32` assembles it from `pci_conf[offset + 0..3]`.
                // A dword read at 0x18FD..0x18FF straddles the end of the
                // 256-byte config space; the bytes past it read as zero,
                // matching `register_write32`, which drops them.
                let o = (a - 0x1800) as usize;
                u32::from_le_bytes(core::array::from_fn(|index| {
                    self.pci_conf.get(o + index).copied().unwrap_or(0)
                }))
            }
            0x2100 => self.fifo_intr,
            0x2140 => self.fifo_intr_en,
            0x2210 => self.fifo_ramht,
            0x2214 if self.card_type < 0x40 => self.fifo_ramfc,
            0x2218 => self.fifo_ramro,
            0x2220 if self.card_type >= 0x40 => self.fifo_ramfc,
            0x2400 => {
                if self.fifo_cache1_get != self.fifo_cache1_put {
                    0
                } else {
                    0x10
                }
            }
            0x2504 => self.fifo_mode,
            0x3200 => self.fifo_cache1_push0,
            0x3204 => self.fifo_cache1_push1,
            0x3210 => self.fifo_cache1_put,
            0x3214 => {
                if self.fifo_cache1_get != self.fifo_cache1_put {
                    0
                } else {
                    0x10
                }
            }
            0x3220 => self.fifo_cache1_dma_push,
            0x322c => self.fifo_cache1_dma_instance,
            0x3230 => 0x8000_0000, // DMA_CTL
            0x3240 => self.fifo_cache1_dma_put,
            0x3244 => self.fifo_cache1_dma_get,
            0x3248 => self.fifo_cache1_ref_cnt,
            0x3250 => self.fifo_cache1_pull0,
            0x3270 => self.fifo_cache1_get,
            0x32e0 => self.fifo_grctx_instance,
            0x3304 => 0x0000_0001,
            0x9100 => self.timer_intr,
            0x9140 => self.timer_intr_en,
            0x9200 => self.timer_num,
            0x9210 => self.timer_den,
            0x9400 => self.get_current_time() as u32,
            0x9410 => (self.get_current_time() >> 32) as u32,
            0x9420 => self.timer_alarm,
            0x10020c => self.memsize,
            0x100320 => match self.card_type {
                0x20 => 0x0000_7fff,
                0x35 => 0x0005_c7ff,
                _ => 0x0002_e3ff,
            },
            0x101000 => self.straps0_primary,
            0x400100 => self.graph_intr,
            0x400108 => self.graph_nsource,
            a if (a == 0x40013C && self.card_type >= 0x40)
                || (a == 0x400140 && self.card_type < 0x40) =>
            {
                self.graph_intr_en
            }
            0x40014C => self.graph_ctx_switch1,
            0x400150 => self.graph_ctx_switch2,
            0x400158 => self.graph_ctx_switch4,
            0x40032c => self.graph_ctxctl_cur,
            0x400700 => self.graph_status,
            0x400704 => self.graph_trapped_addr,
            0x400708 => self.graph_trapped_data,
            0x400718 => self.graph_notify,
            0x400720 => self.graph_fifo,
            0x400724 => self.graph_bpixel,
            0x400780 => self.graph_channel_ctx_table,
            a if (a == 0x400640 && self.card_type == 0x15)
                || (a == 0x400820 && self.card_type == 0x20) =>
            {
                self.graph_offset0
            }
            a if (a == 0x400670 && self.card_type == 0x15)
                || (a == 0x400850 && self.card_type == 0x20) =>
            {
                self.graph_pitch0
            }
            0x600100 => self.crtc_intr,
            0x600140 => self.crtc_intr_en,
            0x600800 => self.crtc_start,
            0x600804 => self.crtc_config,
            0x600808 => 0, // raster pos stub
            0x60080c => self.crtc_cursor_offset,
            0x600810 => self.crtc_cursor_config,
            0x60081c => self.crtc_gpio_ext,
            0x680300 => self.ramdac_cu_start_pos,
            0x680404 => 0, // CURSYNC
            0x680508 => self.ramdac_vpll,
            0x68050c => self.ramdac_pll_select,
            0x680578 => self.ramdac_vpll_b,
            0x680600 => self.ramdac_general_control,
            0x680828 => 0, // FP_HCRTC - second monitor disconnected
            a if a >= 0x700000 && a < 0x800000 => {
                let offset = a & 0x000f_ffff;
                self.ramin_read32(offset)
            }
            _ => {
                let idx = (address / 4) as usize;
                if idx < self.unk_regs.len() {
                    self.unk_regs[idx]
                } else {
                    0
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Register write (MMIO dispatch)
    // -----------------------------------------------------------------------

    pub fn register_write32(&mut self, address: u32, value: u32) {
        match address {
            0x100 => {
                self.mc_soft_intr = (value >> 31) != 0;
                self.update_irq_level();
            }
            0x140 => {
                self.mc_intr_en = value;
                self.update_irq_level();
            }
            0x200 => {
                self.mc_enable = value;
            }
            a if a >= 0x1800 && a < 0x1900 => {
                // PCI config space write
                let offset = (a - 0x1800) as usize;
                let bytes = value.to_le_bytes();
                for i in 0..4 {
                    if offset + i < 256 {
                        self.pci_conf[offset + i] = bytes[i];
                    }
                }
            }
            0x1100 => {
                self.bus_intr &= !value;
                self.update_irq_level();
            }
            0x1140 => {
                self.bus_intr_en = value;
                self.update_irq_level();
            }
            0x2100 => {
                self.fifo_intr &= !value;
                self.update_irq_level();
            }
            0x2140 => {
                self.fifo_intr_en = value;
                self.update_irq_level();
            }
            0x2210 => {
                self.fifo_ramht = value;
            }
            0x2214 if self.card_type < 0x40 => {
                self.fifo_ramfc = value;
            }
            0x2218 => {
                self.fifo_ramro = value;
            }
            0x2220 if self.card_type >= 0x40 => {
                self.fifo_ramfc = value;
            }
            0x2504 => {
                let process = (self.fifo_mode | value) != self.fifo_mode;
                self.fifo_mode = value;
                if process {
                    self.fifo_process_all();
                }
            }
            0x3200 => {
                self.fifo_cache1_push0 = value;
                if self.fifo_cache1_push0 & 1 != 0 {
                    self.fifo_process_all();
                }
            }
            0x3204 => {
                self.fifo_cache1_push1 = value;
            }
            0x3210 => {
                self.fifo_cache1_put = value;
            }
            0x3220 => {
                self.fifo_cache1_dma_push = value;
            }
            0x322c => {
                self.fifo_cache1_dma_instance = value;
            }
            0x3240 => {
                self.fifo_cache1_dma_put = value;
            }
            0x3244 => {
                self.fifo_cache1_dma_get = value;
            }
            0x3248 => {
                self.fifo_cache1_ref_cnt = value;
            }
            0x3250 => {
                self.fifo_cache1_pull0 = value;
                if self.fifo_cache1_pull0 & 1 != 0 {
                    self.fifo_process_all();
                }
            }
            0x3270 => {
                self.fifo_cache1_get = value & (GEFORCE_CACHE1_SIZE as u32 * 4 - 1);
                if self.fifo_cache1_get != self.fifo_cache1_put {
                    self.fifo_intr |= 0x0000_0001;
                } else {
                    self.fifo_intr &= !0x0000_0001;
                    self.fifo_cache1_pull0 &= !0x0000_0100;
                    if self.fifo_wait_soft {
                        self.fifo_wait_soft = false;
                        self.update_fifo_wait();
                        self.fifo_process_all();
                    }
                }
                self.update_irq_level();
            }
            0x32e0 => {
                self.fifo_grctx_instance = value;
            }
            0x9100 => {
                self.timer_intr &= !value;
            }
            0x9140 => {
                self.timer_intr_en = value;
            }
            0x9200 => {
                self.timer_num = value;
            }
            0x9210 => {
                self.timer_den = value;
            }
            0x9400 | 0x9410 => {
                self.timer_inittime2 = self.time_nsec;
                if address == 0x9400 {
                    self.timer_inittime1 =
                        (self.timer_inittime1 & 0xFFFF_FFFF_0000_0000) | value as u64;
                } else {
                    self.timer_inittime1 =
                        (self.timer_inittime1 & 0x0000_0000_FFFF_FFFF) | ((value as u64) << 32);
                }
            }
            0x9420 => {
                self.timer_alarm = value;
            }
            0x101000 => {
                if value >> 31 != 0 {
                    self.straps0_primary = value;
                } else {
                    self.straps0_primary = self.straps0_primary_original;
                }
            }
            0x400100 => {
                self.graph_intr &= !value;
                self.update_irq_level();
                if self.fifo_wait_notify && self.graph_intr == 0 {
                    self.fifo_wait_notify = false;
                    self.update_fifo_wait();
                    self.fifo_process_all();
                }
            }
            0x400108 => {
                self.graph_nsource = value;
            }
            a if (a == 0x40013C && self.card_type >= 0x40)
                || (a == 0x400140 && self.card_type < 0x40) =>
            {
                self.graph_intr_en = value;
                self.update_irq_level();
            }
            0x40014C => {
                self.graph_ctx_switch1 = value;
            }
            0x400150 => {
                self.graph_ctx_switch2 = value;
            }
            0x400158 => {
                self.graph_ctx_switch4 = value;
            }
            0x40032c => {
                self.graph_ctxctl_cur = value;
            }
            0x400700 => {
                self.graph_status = value;
            }
            0x400704 => {
                self.graph_trapped_addr = value;
            }
            0x400708 => {
                self.graph_trapped_data = value;
            }
            0x400718 => {
                self.graph_notify = value;
            }
            0x40071c => {
                if value & 2 != 0 {
                    self.graph_flip_read += 1;
                    if self.graph_flip_modulo > 0 {
                        self.graph_flip_read %= self.graph_flip_modulo;
                    }
                    if self.fifo_wait_flip && self.graph_flip_read != self.graph_flip_write {
                        self.fifo_wait_flip = false;
                        self.update_fifo_wait();
                        self.fifo_process_all();
                    }
                }
            }
            0x400720 => {
                self.graph_fifo = value;
            }
            0x400724 => {
                self.graph_bpixel = value;
            }
            0x400780 => {
                self.graph_channel_ctx_table = value;
            }
            a if (a == 0x400640 && self.card_type == 0x15)
                || (a == 0x400820 && self.card_type == 0x20) =>
            {
                self.graph_offset0 = value;
            }
            a if (a == 0x400670 && self.card_type == 0x15)
                || (a == 0x400850 && self.card_type == 0x20) =>
            {
                self.graph_pitch0 = value;
            }
            0x600100 => {
                self.crtc_intr &= !value;
                self.update_irq_level();
            }
            0x600140 => {
                self.crtc_intr_en = value;
                self.update_irq_level();
            }
            0x600800 => {
                self.crtc_start = value;
                self.svga_needs_update_mode = true;
            }
            0x600804 => {
                self.crtc_config = value;
            }
            0x60080c => {
                self.crtc_cursor_offset = value;
                self.hw_cursor.offset = self.crtc_cursor_offset;
            }
            0x600810 => {
                self.crtc_cursor_config = value;
                self.hw_cursor.enabled = (self.crtc.reg[0x31] & 0x01 != 0) || (value & 1 != 0);
                self.hw_cursor.vram = (self.crtc.reg[0x30] & 0x80 != 0)
                    || (value & 0x0000_0100 != 0)
                    || (self.card_type >= 0x40);
                self.hw_cursor.size = if value & 0x0001_0000 != 0 { 64 } else { 32 };
                self.hw_cursor.bpp32 = value & 0x0000_1000 != 0;
            }
            0x60081c => {
                self.crtc_gpio_ext = value;
            }
            0x680300 => {
                self.ramdac_cu_start_pos = value;
                self.hw_cursor.x = ((value as i32) << 20 >> 20) as i16;
                self.hw_cursor.y = ((value as i32) << 4 >> 20) as i16;
            }
            0x680508 => {
                self.ramdac_vpll = value;
            }
            0x68050c => {
                self.ramdac_pll_select = value;
            }
            0x680578 => {
                self.ramdac_vpll_b = value;
            }
            0x680600 => {
                self.ramdac_general_control = value;
            }
            a if a >= 0x700000 && a < 0x800000 => {
                self.ramin_write32(a - 0x700000, value);
            }
            a if (a >= 0x800000 && a < 0xA00000) || (a >= 0xC00000 && a < 0xE00000) => {
                let (chid, offset) = if a >= 0x800000 && a < 0xA00000 {
                    (((a >> 16) & 0x1F) as usize, a & 0x1FFF)
                } else {
                    let ch = ((a >> 12) & 0x1FF) as usize;
                    (ch.min(GEFORCE_CHANNEL_COUNT - 1), a & 0x1FF)
                };
                if self.fifo_mode & (1 << chid) != 0 {
                    if offset == 0x40 {
                        let curchid = (self.fifo_cache1_push1 & 0x1F) as usize;
                        if curchid == chid {
                            self.fifo_cache1_dma_put = value;
                        } else {
                            self.ramfc_write32(chid as u32, 0x0, value);
                        }
                        self.fifo_process_channel(chid as u32);
                    }
                } else if a >= 0x800000 && a < 0xA00000 {
                    let subc = ((a >> 13) & 7) as u32;
                    self.execute_command(chid as u32, subc, offset / 4, value);
                }
            }
            _ => {
                let idx = (address / 4) as usize;
                if idx < self.unk_regs.len() {
                    self.unk_regs[idx] = value;
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // FIFO processing
    // -----------------------------------------------------------------------

    pub fn fifo_process_all(&mut self) {
        let offset = (self.fifo_cache1_push1 & 0x1f) + 1;
        for i in 0..GEFORCE_CHANNEL_COUNT as u32 {
            self.fifo_process_channel((i + offset) & 0x1f);
        }
    }

    pub fn fifo_process_channel(&mut self, chid: u32) {
        if self.fifo_wait {
            return;
        }
        if self.fifo_mode & (1 << chid) == 0 {
            return;
        }
        if self.fifo_cache1_push0 & 1 == 0 {
            return;
        }
        if self.fifo_cache1_pull0 & 1 == 0 {
            return;
        }

        let oldchid = self.fifo_cache1_push1 & 0x1F;
        if oldchid == chid {
            if self.fifo_cache1_dma_put == self.fifo_cache1_dma_get {
                return;
            }
        } else {
            if self.ramfc_read32(chid, 0x0) == self.ramfc_read32(chid, 0x4) {
                return;
            }
        }

        // Channel context switch
        if oldchid != chid {
            self.ramfc_write32(oldchid, 0x0, self.fifo_cache1_dma_put);
            self.ramfc_write32(oldchid, 0x4, self.fifo_cache1_dma_get);
            self.ramfc_write32(oldchid, 0x8, self.fifo_cache1_ref_cnt);
            self.ramfc_write32(oldchid, 0xC, self.fifo_cache1_dma_instance);
            if self.card_type >= 0x20 {
                let sro = if self.card_type < 0x40 { 0x2C } else { 0x30 };
                self.ramfc_write32(oldchid, sro, self.fifo_cache1_semaphore);
            }
            if self.card_type >= 0x40 {
                self.ramfc_write32(oldchid, 0x38, self.fifo_grctx_instance);
            }

            self.fifo_cache1_dma_put = self.ramfc_read32(chid, 0x0);
            self.fifo_cache1_dma_get = self.ramfc_read32(chid, 0x4);
            self.fifo_cache1_ref_cnt = self.ramfc_read32(chid, 0x8);
            self.fifo_cache1_dma_instance = self.ramfc_read32(chid, 0xC);
            if self.card_type >= 0x20 {
                let sro = if self.card_type < 0x40 { 0x2C } else { 0x30 };
                self.fifo_cache1_semaphore = self.ramfc_read32(chid, sro);
            }
            if self.card_type >= 0x40 {
                self.fifo_grctx_instance = self.ramfc_read32(chid, 0x38);
                self.graph_ctxctl_cur = self.fifo_grctx_instance | 0x0100_0000;
            }
            self.fifo_cache1_push1 = (self.fifo_cache1_push1 & !0x1F) | chid;
        }

        self.fifo_cache1_dma_push |= 0x100;
        if self.fifo_cache1_dma_instance == 0 {
            tracing::error!("FIFO: DMA instance = 0");
            return;
        }

        while self.fifo_cache1_dma_get != self.fifo_cache1_dma_put {
            let word =
                self.dma_read32(self.fifo_cache1_dma_instance << 4, self.fifo_cache1_dma_get);
            self.fifo_cache1_dma_get += 4;

            let mcnt = self.chs[chid as usize].dma_state.mcnt;
            if mcnt > 0 {
                let mthd = self.chs[chid as usize].dma_state.mthd;
                let subc = self.chs[chid as usize].dma_state.subc;
                let ni = self.chs[chid as usize].dma_state.ni;
                let cmd_result = self.execute_command(chid, subc, mthd, word);
                if cmd_result <= 1 {
                    if !ni {
                        self.chs[chid as usize].dma_state.mthd += 1;
                    }
                    self.chs[chid as usize].dma_state.mcnt -= 1;
                } else {
                    self.fifo_cache1_dma_get -= 4;
                }
                if cmd_result != 0 {
                    break;
                }
            } else {
                if (word & 0xe000_0003) == 0x2000_0000 {
                    // old jump
                    self.fifo_cache1_dma_get = word & 0x1fff_ffff;
                } else if (word & 3) == 1 {
                    // jump
                    self.fifo_cache1_dma_get = word & 0xffff_fffc;
                } else if (word & 3) == 2 {
                    // call
                    self.chs[chid as usize].subr_return = self.fifo_cache1_dma_get;
                    self.chs[chid as usize].subr_active = true;
                    self.fifo_cache1_dma_get = word & 0xffff_fffc;
                } else if word == 0x0002_0000 {
                    // return
                    self.fifo_cache1_dma_get = self.chs[chid as usize].subr_return;
                    self.chs[chid as usize].subr_active = false;
                } else if (word & 0xa003_0003) == 0 {
                    // method header
                    self.chs[chid as usize].dma_state.mthd = (word >> 2) & 0x7ff;
                    self.chs[chid as usize].dma_state.subc = (word >> 13) & 7;
                    self.chs[chid as usize].dma_state.mcnt = (word >> 18) & 0x7ff;
                    self.chs[chid as usize].dma_state.ni = word & 0x4000_0000 != 0;
                } else {
                    tracing::error!("FIFO: unexpected word {:#010x}", word);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Command execution
    // -----------------------------------------------------------------------

    /// Execute a single GPU command method.
    /// Returns 0 = continue, 1 = stop processing, 2 = retry
    pub fn execute_command(&mut self, chid: u32, subc: u32, method: u32, param: u32) -> i32 {
        let mut result = 0i32;
        let chi = chid as usize;
        let si = subc as usize;

        if method == 0x000 {
            // Subchannel assignment
            let (object, engine) = self.ramht_lookup(param, chid);
            self.chs[chi].schs[si].object = object;
            self.chs[chi].schs[si].engine = engine;

            if engine == 0x01 {
                let word1 = self.ramin_read32(object + 4);
                if self.card_type < 0x40 {
                    self.chs[chi].schs[si].notifier = word1 >> 16 << 4;
                } else {
                    self.chs[chi].schs[si].notifier = (word1 & 0xFFFFF) << 4;
                }
                let word0 = self.ramin_read32(object);
                let cls8 = (word0 & 0xFF) as u8;
                if cls8 == 0x96 || cls8 == 0x97 {
                    self.execute_d3d_init(chi, word0 & self.class_mask);
                }
            } else if engine == 0x00 {
                // Software method
                self.fifo_wait_soft = true;
                self.fifo_wait = true;
                self.fifo_intr |= 0x0000_0001;
                self.update_irq_level();
                result = 1;
            }
        } else if method == 0x014 {
            self.fifo_cache1_ref_cnt = param;
        } else if method >= 0x040 {
            let engine = self.chs[chi].schs[si].engine;
            if engine == 0x01 {
                let mut adjusted_param = param;
                if method >= 0x060 && method < 0x080 {
                    let (obj, _) = self.ramht_lookup(param, chid);
                    adjusted_param = obj;
                }
                let cls = self.ramin_read32(self.chs[chi].schs[si].object) & self.class_mask;
                let cls8 = (cls & 0xFF) as u8;

                match cls8 {
                    0x19 => self.execute_clip(chi, method, adjusted_param),
                    0x39 => self.execute_m2mf(chi, si, method, adjusted_param),
                    0x43 => self.execute_rop(chi, method, adjusted_param),
                    0x44 | 0x18 => self.execute_patt(chi, method, adjusted_param),
                    0x4a | 0x4b => self.execute_gdi(chi, cls, method, adjusted_param),
                    0x52 | 0x9e => self.execute_swzsurf(chi, method, adjusted_param),
                    0x57 => self.execute_chroma(chi, method, adjusted_param),
                    0x5e => self.execute_rect(chi, method, adjusted_param),
                    0x5f | 0x9f => self.execute_imageblit(chi, method, adjusted_param),
                    0x61 | 0x65 | 0x8a | 0x21 => self.execute_ifc(chi, method, adjusted_param),
                    0x62 => self.execute_surf2d(chi, method, adjusted_param),
                    0x64 => self.execute_iifc(chi, method, adjusted_param),
                    0x66 | 0x76 => self.execute_sifc(chi, method, adjusted_param),
                    0x72 => self.execute_beta(chi, method, adjusted_param),
                    0x7b => self.execute_tfc(chi, method, adjusted_param),
                    0x89 => self.execute_sifm(chi, cls, method, adjusted_param),
                    0x96 | 0x97 => {
                        self.execute_d3d(chi, cls, method, adjusted_param);
                        if self.fifo_wait_flip {
                            result = 1;
                        }
                    }
                    _ => {
                        tracing::debug!("Unknown object class {:#04x}", cls8);
                    }
                }

                // Handle notify
                if self.chs[chi].notify_pending {
                    self.chs[chi].notify_pending = false;
                    let notifier = self.chs[chi].schs[si].notifier;
                    let t = self.get_current_time();
                    self.dma_write64(notifier, 0x0, t);
                    self.dma_write32(notifier, 0x8, 0);
                    self.dma_write32(notifier, 0xC, 0);
                }

                if method == 0x041 {
                    self.chs[chi].notify_pending = true;
                    self.chs[chi].notify_type = adjusted_param;
                } else if method == 0x060 {
                    self.chs[chi].schs[si].notifier = adjusted_param;
                }
            } else if engine == 0x00 {
                self.fifo_wait_soft = true;
                self.fifo_wait = true;
                self.fifo_intr |= 0x0000_0001;
                self.update_irq_level();
                result = 1;
            }
        }

        result
    }

    // -----------------------------------------------------------------------
    // 2D command handlers
    // -----------------------------------------------------------------------

    fn execute_clip(&mut self, chi: usize, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        if method == 0x0c0 {
            ch.clip_x = param as u16;
            ch.clip_y = (param >> 16) as u16;
        } else if method == 0x0c1 {
            ch.clip_width = param as u16;
            ch.clip_height = (param >> 16) as u16;
        }
    }

    fn execute_m2mf(&mut self, chi: usize, si: usize, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        match method {
            0x061 => ch.m2mf_src = param,
            0x062 => ch.m2mf_dst = param,
            0x0c3 => ch.m2mf_src_offset = param,
            0x0c4 => ch.m2mf_dst_offset = param,
            0x0c5 => ch.m2mf_src_pitch = param,
            0x0c6 => ch.m2mf_dst_pitch = param,
            0x0c7 => ch.m2mf_line_length = param,
            0x0c8 => ch.m2mf_line_count = param,
            0x0c9 => ch.m2mf_format = param,
            0x0ca => {
                ch.m2mf_buffer_notify = param;
                // Execute M2MF
                let src = ch.m2mf_src;
                let dst = ch.m2mf_dst;
                let mut src_offset = ch.m2mf_src_offset;
                let mut dst_offset = ch.m2mf_dst_offset;
                let src_pitch = ch.m2mf_src_pitch;
                let dst_pitch = ch.m2mf_dst_pitch;
                let line_length = ch.m2mf_line_length;
                let line_count = ch.m2mf_line_count;
                for _ in 0..line_count {
                    self.dma_copy(dst, dst_offset, src, src_offset, line_length);
                    src_offset += src_pitch;
                    dst_offset += dst_pitch;
                }
                // Notify
                let notifier = self.chs[chi].schs[si].notifier;
                let t = self.get_current_time();
                self.dma_write64(notifier, 0x10, t);
                self.dma_write32(notifier, 0x18, 0);
                self.dma_write32(notifier, 0x1C, 0);
            }
            _ => {}
        }
    }

    fn execute_rop(&mut self, chi: usize, method: u32, param: u32) {
        if method == 0x0c0 {
            self.chs[chi].rop = param as u8;
        }
    }

    fn execute_patt(&mut self, chi: usize, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        match method {
            0x0c2 => ch.patt_shape = param,
            0x0c3 => ch.patt_type_color = param == 2,
            0x0c4 => ch.patt_bg_color = param,
            0x0c5 => ch.patt_fg_color = param,
            0x0c6 | 0x0c7 => {
                let base = (method & 1) as usize * 32;
                for i in 0..32 {
                    ch.patt_data_mono[i + base] = (1 << (i ^ 7)) & param != 0;
                }
            }
            m if m >= 0x100 && m < 0x110 => {
                let i = ((m - 0x100) * 4) as usize;
                ch.patt_data_color[i] = param & 0xFF;
                ch.patt_data_color[i + 1] = (param >> 8) & 0xFF;
                ch.patt_data_color[i + 2] = (param >> 16) & 0xFF;
                ch.patt_data_color[i + 3] = param >> 24;
            }
            m if m >= 0x140 && m < 0x160 => {
                let i = ((m - 0x140) * 2) as usize;
                ch.patt_data_color[i] = param & 0xFFFF;
                ch.patt_data_color[i + 1] = param >> 16;
            }
            m if m >= 0x1c0 && m < 0x200 => {
                ch.patt_data_color[(m - 0x1c0) as usize] = param;
            }
            _ => {}
        }
    }

    fn execute_gdi(&mut self, chi: usize, cls: u32, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        match method {
            0x0bf => ch.gdi_operation = param,
            0x0c0 => ch.gdi_color_fmt = param,
            0x0c1 => ch.gdi_mono_fmt = param,
            0x0ff => ch.gdi_rect_color = param,
            m if m >= 0x100 && m < 0x140 => {
                if m & 1 != 0 {
                    ch.gdi_rect_wh = param;
                    // gdi_fillrect would execute here
                } else {
                    ch.gdi_rect_xy = param;
                }
            }
            0x17d => ch.gdi_clip_yx0 = param,
            0x17e => ch.gdi_clip_yx1 = param,
            0x17f => ch.gdi_rect_color = param,
            m if m >= 0x180 && m < 0x1c0 => {
                if m & 1 != 0 {
                    ch.gdi_rect_yx1 = param;
                    // gdi_fillrect clipped would execute here
                } else {
                    ch.gdi_rect_yx0 = param;
                }
            }
            _ => {
                // Additional GDI methods handled per cls
                tracing::debug!("GDI method {:#05x} cls {:#06x}", method, cls);
            }
        }
    }

    fn execute_swzsurf(&mut self, chi: usize, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        match method {
            0x061 => ch.swzs_img_obj = param,
            0x0c0 => {
                ch.swzs_fmt = param;
                ch.swzs_width = 1 << ((param >> 16) & 0xff);
                ch.swzs_height = 1 << (param >> 24);
                let color_fmt = param & 0xffff;
                ch.swzs_color_bytes = match color_fmt {
                    1 => 1,
                    2 | 4 => 2,
                    6 | 0xA | 0xB => 4,
                    _ => {
                        tracing::error!(
                            "unknown swizzled surface color format: {:#04x}",
                            color_fmt
                        );
                        1
                    }
                };
            }
            0x0c1 => ch.swzs_ofs = param,
            _ => {}
        }
    }

    fn execute_chroma(&mut self, chi: usize, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        if method == 0x0c0 {
            ch.chroma_color_fmt = param;
        } else if method == 0x0c1 {
            ch.chroma_color = param;
        }
    }

    fn execute_rect(&mut self, chi: usize, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        match method {
            0x0bf => ch.rect_operation = param,
            0x0c0 => ch.rect_color_fmt = param,
            0x0c1 => ch.rect_color = param,
            m if m >= 0x100 && m < 0x120 => {
                if m & 1 != 0 {
                    ch.rect_hw = param;
                    // rect() would execute here
                } else {
                    ch.rect_yx = param;
                }
            }
            _ => {}
        }
    }

    fn execute_imageblit(&mut self, chi: usize, method: u32, param: u32) {
        match method {
            0x061 => {
                self.chs[chi].blit_color_key_enable = (self.ramin_read32(param) & 0xFF) != 0x30;
                return;
            }
            _ => {}
        }
        let ch = &mut self.chs[chi];
        match method {
            0x0bf => ch.blit_operation = param,
            0x0c0 => ch.blit_syx = param,
            0x0c1 => ch.blit_dyx = param,
            0x0c2 => {
                ch.blit_hw = param;
                // copyarea() would execute here
            }
            _ => {}
        }
    }

    fn execute_ifc(&mut self, chi: usize, method: u32, param: u32) {
        match method {
            0x061 => {
                self.chs[chi].ifc_color_key_enable = (self.ramin_read32(param) & 0xFF) != 0x30;
                return;
            }
            0x062 => {
                self.chs[chi].ifc_clip_enable = (self.ramin_read32(param) & 0xFF) != 0x30;
                return;
            }
            _ => {}
        }
        let ch = &mut self.chs[chi];
        match method {
            0x0bf => ch.ifc_operation = param,
            0x0c0 => {
                ch.ifc_color_fmt = param;
                Self::update_color_bytes(
                    ch.s2d_color_fmt,
                    ch.ifc_color_fmt,
                    &mut ch.ifc_color_bytes,
                );
                ch.ifc_pixels_per_word = 4 / ch.ifc_color_bytes;
            }
            0x0c1 => {
                ch.ifc_x = 0;
                ch.ifc_y = 0;
                ch.ifc_ofs_x = param & 0xFFFF;
                ch.ifc_ofs_y = param >> 16;
                ch.ifc_draw_offset = ch.s2d_ofs_dst
                    + ch.ifc_ofs_y * ch.s2d_pitch_dst
                    + ch.ifc_ofs_x * ch.s2d_color_bytes;
            }
            0x0c2 => {
                ch.ifc_dst_width = param & 0xFFFF;
                ch.ifc_dst_height = param >> 16;
                ch.ifc_clip_x0 = 0;
                ch.ifc_clip_y0 = 0;
                ch.ifc_clip_x1 = ch.ifc_dst_width;
                ch.ifc_clip_y1 = ch.ifc_dst_height;
            }
            0x0c3 => {
                ch.ifc_src_width = param & 0xFFFF;
                ch.ifc_src_height = param >> 16;
            }
            _ => {
                // Data words (0x100..0x800) would call ifc(ch, param)
            }
        }
    }

    fn execute_surf2d(&mut self, chi: usize, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        ch.s2d_locked = true;
        match method {
            0x061 => ch.s2d_img_src = param,
            0x062 => ch.s2d_img_dst = param,
            0x0c0 => {
                ch.s2d_color_fmt = param;
                Self::update_color_bytes_s2d(ch);
            }
            0x0c1 => {
                ch.s2d_pitch_src = param & 0xFFFF;
                ch.s2d_pitch_dst = param >> 16;
            }
            0x0c2 => ch.s2d_ofs_src = param,
            0x0c3 => ch.s2d_ofs_dst = param,
            _ => {}
        }
    }

    fn execute_iifc(&mut self, chi: usize, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        match method {
            0x061 => ch.iifc_palette = param,
            0x0f9 => ch.iifc_operation = param,
            0x0fa => {
                ch.iifc_color_fmt = param;
                Self::update_color_bytes(0, ch.iifc_color_fmt, &mut ch.iifc_color_bytes);
            }
            0x0fb => ch.iifc_bpp4 = param,
            0x0fc => ch.iifc_palette_ofs = param,
            0x0fd => ch.iifc_yx = param,
            0x0fe => ch.iifc_dhw = param,
            0x0ff => {
                ch.iifc_shw = param;
                let width = ch.iifc_shw & 0xFFFF;
                let height = ch.iifc_shw >> 16;
                let bpp = if ch.iifc_bpp4 != 0 { 4 } else { 8 };
                let word_count = align_up(width * height * bpp, 32) >> 5;
                ch.iifc_words_ptr = 0;
                ch.iifc_words_left = word_count;
                ch.iifc_words = Some(vec![0u32; word_count as usize]);
            }
            m if m >= 0x100 && m < 0x800 => {
                if let Some(ref mut words) = ch.iifc_words {
                    if (ch.iifc_words_ptr as usize) < words.len() {
                        words[ch.iifc_words_ptr as usize] = param;
                        ch.iifc_words_ptr += 1;
                        ch.iifc_words_left -= 1;
                        if ch.iifc_words_left == 0 {
                            // iifc() would execute here
                            ch.iifc_words = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn execute_sifc(&mut self, chi: usize, method: u32, param: u32) {
        let ch = &mut self.chs[chi];
        match method {
            0x0bf => ch.sifc_operation = param,
            0x0c0 => {
                ch.sifc_color_fmt = param;
                Self::update_color_bytes(
                    ch.s2d_color_fmt,
                    ch.sifc_color_fmt,
                    &mut ch.sifc_color_bytes,
                );
            }
            0x0c1 => ch.sifc_shw = param,
            0x0c2 => ch.sifc_dxds = param,
            0x0c3 => ch.sifc_dydt = param,
            0x0c4 => ch.sifc_clip_yx = param,
            0x0c5 => ch.sifc_clip_hw = param,
            0x0c6 => {
                ch.sifc_syx = param;
                let width = ch.sifc_shw & 0xFFFF;
                let height = ch.sifc_shw >> 16;
                let word_count = align_up(width * height * ch.sifc_color_bytes, 4) >> 2;
                ch.sifc_words_ptr = 0;
                ch.sifc_words_left = word_count;
                ch.sifc_words = Some(vec![0u32; word_count as usize]);
            }
            m if m >= 0x100 && m < 0x800 => {
                if let Some(ref mut words) = ch.sifc_words {
                    if (ch.sifc_words_ptr as usize) < words.len() {
                        words[ch.sifc_words_ptr as usize] = param;
                        ch.sifc_words_ptr += 1;
                        ch.sifc_words_left -= 1;
                        if ch.sifc_words_left == 0 {
                            // sifc() would execute here
                            ch.sifc_words = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn execute_beta(&mut self, chi: usize, method: u32, param: u32) {
        if method == 0x0c0 {
            self.chs[chi].beta = param;
        }
    }

    fn execute_tfc(&mut self, chi: usize, method: u32, param: u32) {
        match method {
            0x061 => {
                let cls8 = self.ramin_read32(param) as u8;
                self.chs[chi].tfc_swizzled = cls8 == 0x52 || cls8 == 0x9e;
                return;
            }
            _ => {}
        }
        let ch = &mut self.chs[chi];
        match method {
            0x0c0 => {
                ch.tfc_color_fmt = param;
                Self::update_color_bytes(
                    ch.s2d_color_fmt,
                    ch.tfc_color_fmt,
                    &mut ch.tfc_color_bytes,
                );
            }
            0x0c1 => ch.tfc_yx = param,
            0x0c2 => {
                ch.tfc_hw = param;
                ch.tfc_upload = param == 0x0100_0100
                    && ch.tfc_yx == 0
                    && ch.tfc_color_fmt == 4
                    && ch.s2d_color_fmt == 0xA
                    && ch.s2d_pitch_src == 0x0400
                    && ch.s2d_pitch_dst == 0x0400;
                if ch.tfc_upload {
                    ch.tfc_upload_offset = ch.s2d_ofs_dst;
                } else {
                    let width = ch.tfc_hw & 0xFFFF;
                    let height = ch.tfc_hw >> 16;
                    let word_count = align_up(width * height * ch.tfc_color_bytes, 4) >> 2;
                    ch.tfc_words_ptr = 0;
                    ch.tfc_words_left = word_count;
                    ch.tfc_words = Some(vec![0u32; word_count as usize]);
                }
            }
            0x0c3 => ch.tfc_clip_wx = param,
            0x0c4 => ch.tfc_clip_hy = param,
            m if m >= 0x100 && m < 0x800 => {
                if ch.tfc_upload {
                    let s2d_img_dst = ch.s2d_img_dst;
                    let ofs = ch.tfc_upload_offset;
                    self.dma_write32(s2d_img_dst, ofs, param);
                    self.chs[chi].tfc_upload_offset += 4;
                } else if let Some(ref mut words) = ch.tfc_words {
                    if (ch.tfc_words_ptr as usize) < words.len() {
                        words[ch.tfc_words_ptr as usize] = param;
                        ch.tfc_words_ptr += 1;
                        ch.tfc_words_left -= 1;
                        if ch.tfc_words_left == 0 {
                            // tfc() would execute here
                            ch.tfc_words = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn execute_sifm(&mut self, chi: usize, cls: u32, method: u32, param: u32) {
        match method {
            0x066 => {
                let surf_cls8 = self.ramin_read32(param) as u8;
                let swizzled = surf_cls8 == 0x52 || surf_cls8 == 0x9e;
                if cls == 0x0389 {
                    self.chs[chi].sifm_swizzled_0389 = swizzled;
                } else {
                    self.chs[chi].sifm_swizzled = swizzled;
                }
                return;
            }
            _ => {}
        }
        let ch = &mut self.chs[chi];
        match method {
            0x0c0 => {
                ch.sifm_color_fmt = param;
                ch.sifm_color_bytes = match param {
                    8 => 1,
                    1 | 2 | 7 => 2,
                    3 | 4 => 4,
                    _ => {
                        tracing::error!("unknown sifm color format: {:#04x}", param);
                        4
                    }
                };
            }
            0x0c1 => ch.sifm_operation = param,
            0x0c4 => ch.sifm_dyx = param,
            0x0c5 => ch.sifm_dhw = param,
            0x0c6 => ch.sifm_dudx = param as i32,
            0x0c7 => ch.sifm_dvdy = param as i32,
            0x100 => ch.sifm_shw = param,
            0x101 => ch.sifm_sfmt = param,
            0x102 => ch.sifm_sofs = param,
            0x103 => {
                ch.sifm_syx = param;
                // sifm() would execute here
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Color bytes helpers
    // -----------------------------------------------------------------------

    fn update_color_bytes_s2d(ch: &mut GfChannel) {
        ch.s2d_color_bytes = match ch.s2d_color_fmt {
            0x1 => 1,                   // Y8
            0x2 | 0x4 | 0x5 => 2,       // X1R5G5B5/R5G6B5/Y16
            0x6 | 0x7 | 0xA | 0xB => 4, // X8R8G8B8/A8R8G8B8/Y32
            _ => {
                tracing::error!("unknown 2d surface color format: {:#04x}", ch.s2d_color_fmt);
                1
            }
        };
    }

    fn update_color_bytes(s2d_color_fmt: u32, color_fmt: u32, color_bytes: &mut u32) {
        if s2d_color_fmt == 1 {
            *color_bytes = 1; // hack for Y8
        } else {
            *color_bytes = match color_fmt {
                1 | 2 | 3 => 2, // R5G6B5/A1R5G5B5/X1R5G5B5
                4 | 5 => 4,     // A8R8G8B8/X8R8G8B8
                _ => {
                    tracing::error!("unknown color format: {:#04x}", color_fmt);
                    4
                }
            };
        }
    }

    // -----------------------------------------------------------------------
    // 3D command handler (stub dispatches to execute_d3d_init + methods)
    // -----------------------------------------------------------------------

    fn execute_d3d_init(&mut self, chi: usize, cls: u32) {
        let ch = &mut self.chs[chi];
        if cls == 0x0096 {
            ch.d3d_window_offset_x = 2048;
            ch.d3d_window_offset_y = 2048;
            ch.d3d_attrib_count = 8;
        } else {
            ch.d3d_window_offset_x = 0;
            ch.d3d_window_offset_y = 0;
            ch.d3d_attrib_count = 16;
        }
        for j in 0..ch.d3d_attrib_count as usize {
            ch.d3d_vertex_data_array_format_type[j] = 0;
            ch.d3d_vertex_data_array_format_size[j] = 0;
            ch.d3d_vertex_data_array_format_stride[j] = 0;
            ch.d3d_vertex_data_array_format_dx[j] = false;
            ch.d3d_vertex_data_array_format_homogeneous[j] = false;
        }
        ch.d3d_vs_temp_regs_count = match cls {
            0x0096 => 0,
            0x0097 => 12,
            c if c <= 0x0497 => 16,
            _ => 32,
        };
        ch.d3d_tex_coord_count = match cls {
            0x0096 => 2,
            0x0097 => 4,
            _ => 8,
        };
        if cls == 0x0096 {
            ch.d3d_attrib_in_color = [1, 2];
            ch.d3d_attrib_in_normal = 5;
            ch.d3d_combiner_control_num_stages = 2;
        } else {
            ch.d3d_attrib_in_color = [3, 4];
            ch.d3d_attrib_in_normal = 2;
        }
        ch.d3d_attrib_out_color = [3, 4];
        ch.d3d_attrib_out_fogc = 5;
        ch.d3d_attrib_out_enable = [true; 32];
        ch.d3d_attrib_in_tex_coord = [0xf; 16];
        ch.d3d_attrib_out_tex_coord = [0xf; 16];
        for j in 0..ch.d3d_tex_coord_count as usize {
            ch.d3d_attrib_in_tex_coord[j] = match cls {
                0x0096 => j as u32 + 3,
                0x0097 => j as u32 + 9,
                _ => j as u32 + 8,
            };
            ch.d3d_attrib_out_tex_coord[j] = match cls {
                c if c <= 0x0097 => j as u32 + 9,
                c if c <= 0x0497 => j as u32 + 8,
                _ => j as u32 + 7,
            };
        }
        for ci in 0..4 {
            ch.d3d_vertex_data_imm[ch.d3d_attrib_in_color[0] as usize][ci] = 1.0;
        }
    }

    fn execute_d3d(&mut self, chi: usize, cls: u32, method: u32, param: u32) {
        let param_float = uint32_as_float(param);

        // Object bindings
        match method {
            0x048 => {
                self.graph_flip_read = param;
                return;
            }
            0x049 => {
                self.graph_flip_write = param;
                return;
            }
            0x04a => {
                self.graph_flip_modulo = param;
                return;
            }
            0x04b => {
                self.graph_flip_write += 1;
                if self.graph_flip_modulo > 0 {
                    self.graph_flip_write %= self.graph_flip_modulo;
                }
                return;
            }
            0x04c => {
                if self.graph_flip_read == self.graph_flip_write {
                    self.fifo_wait_flip = true;
                    self.fifo_wait = true;
                }
                return;
            }
            _ => {}
        }

        // Reborrow ch after self borrows above
        let ch = &mut self.chs[chi];

        match method {
            0x061 => ch.d3d_a_obj = param,
            0x062 => ch.d3d_b_obj = param,
            0x063 if cls == 0x0096 => {
                ch.d3d_vertex_a_obj = param;
                ch.d3d_vertex_b_obj = param;
            }
            0x065 => ch.d3d_color_obj = param,
            0x066 => ch.d3d_zeta_obj = param,
            0x067 => ch.d3d_vertex_a_obj = param,
            0x068 => ch.d3d_vertex_b_obj = param,
            0x069 => ch.d3d_semaphore_obj = param,
            0x06a => ch.d3d_report_obj = param,
            0x080 => ch.d3d_clip_horizontal = param,
            0x081 => ch.d3d_clip_vertical = param,
            0x082 => {
                ch.d3d_surface_format = param;
                let (format_color, format_depth) = if cls <= 0x0097 {
                    (param & 0xF, (param >> 4) & 0xF)
                } else {
                    (param & 0x1F, (param >> 5) & 0x7)
                };
                ch.d3d_color_bytes = match format_color {
                    0x9 => 1,
                    0x3 => 2,
                    0x4 | 0x5 | 0x8 => 4,
                    _ => {
                        tracing::error!("unknown D3D color format: {:#03x}", format_color);
                        4
                    }
                };
                ch.d3d_depth_bytes = match format_depth {
                    0 => ch.d3d_color_bytes,
                    1 => 2, // Z16
                    2 => 4, // Z24S8
                    _ => {
                        tracing::error!("unknown D3D depth format: {:#03x}", format_depth);
                        4
                    }
                };
                if cls == 0x0096 {
                    ch.d3d_viewport_scale[2] = if ch.d3d_depth_bytes == 2 {
                        32767.0
                    } else {
                        8388607.0
                    };
                }
            }
            0x083 => ch.d3d_surface_pitch_a = param,
            0x084 => ch.d3d_surface_color_offset = param,
            0x085 => ch.d3d_surface_zeta_offset = param,
            0x08b if cls > 0x0497 => ch.d3d_surface_pitch_z = param,

            // Fog
            m if (m == 0x0a7 && cls <= 0x0097) || (m == 0x233 && cls >= 0x0497) => {
                ch.d3d_fog_mode = param;
            }
            m if (m == 0x0a8 && cls <= 0x0097) || (m == 0x232 && cls >= 0x0497) => {
                ch.d3d_fog_gen_mode = param;
            }
            m if (m == 0x0a9 && cls <= 0x0097) || (m == 0x0db && cls == 0x0497) => {
                ch.d3d_fog_enable = param;
            }

            // Alpha/blend/depth/stencil enable
            m if (m == 0x0c0 && cls <= 0x0097) || (m == 0x0c1 && cls >= 0x0497) => {
                ch.d3d_alpha_test_enable = param;
            }
            m if (m == 0x0c1 && cls <= 0x0097) || (m == 0x0c4 && cls >= 0x0497) => {
                ch.d3d_blend_enable = param;
            }
            m if (m == 0x0c3 && cls <= 0x0097) || (m == 0x29d && cls >= 0x0497) => {
                ch.d3d_depth_test_enable = param;
            }
            m if (m == 0x0c5 && cls <= 0x0097) || (m == 0x516 && cls >= 0x0497) => {
                ch.d3d_lighting_enable = param;
            }

            // Depth function
            m if (m == 0x0d5 && cls <= 0x0097) || (m == 0x29b && cls >= 0x0497) => {
                ch.d3d_depth_func = param;
            }
            m if (m == 0x0d7 && cls <= 0x0097) || (m == 0x29c && cls >= 0x0497) => {
                ch.d3d_depth_write_enable = param;
            }

            // Color mask
            m if (m == 0x0d6 && cls <= 0x0097) || (m == 0x0c9 && cls >= 0x0497) => {
                ch.d3d_color_mask = param;
            }

            // Shade mode
            m if (m == 0x0df && cls <= 0x0097) || (m == 0x0da && cls >= 0x0497) => {
                ch.d3d_shade_mode = param;
            }

            // Clip range
            0x0e5 => ch.d3d_clip_min = param_float,
            0x0e6 => ch.d3d_clip_max = param_float,

            // Cull face
            m if (m == 0x0c2 && cls <= 0x0097) || (m == 0x60f && cls >= 0x0497) => {
                ch.d3d_cull_face_enable = param;
            }
            m if (m == 0x0e7 && cls <= 0x0097) || (m == 0x60c && cls >= 0x0497) => {
                ch.d3d_cull_face = param;
            }
            m if (m == 0x0e8 && cls <= 0x0097) || (m == 0x60d && cls >= 0x0497) => {
                ch.d3d_front_face = param;
            }

            // Window offset (NV30+)
            0x0ae if cls >= 0x0497 => {
                ch.d3d_window_offset_x = param as i16;
                ch.d3d_window_offset_y = (param >> 16) as i16;
            }

            // Clear surface
            0x763 => ch.d3d_zstencil_clear_value = param,
            0x764 => ch.d3d_color_clear_value = param,
            0x765 => {
                ch.d3d_clear_surface = param;
                // d3d_clear_surface() would execute here
            }

            // Transform program
            0x7a5 => ch.d3d_transform_execution_mode = param,
            0x7a7 => ch.d3d_transform_program_load = param,
            0x7a8 => ch.d3d_transform_program_start = param,

            // Begin/end primitive
            m if (m == 0x37f && cls == 0x0096)
                || (m == 0x4ff && cls == 0x0096)
                || (m == 0x5ff && cls <= 0x0097)
                || (m == 0x602 && cls >= 0x0497) =>
            {
                if param != 0 {
                    ch.d3d_primitive_done = false;
                    ch.d3d_triangle_flip = false;
                    ch.d3d_vertex_index = 0;
                    ch.d3d_attrib_index = if cls == 0x0096 { 7 } else { 0 };
                    ch.d3d_comp_index = 0;
                }
                ch.d3d_begin_end = param;
            }

            // Semaphore
            0x75b => ch.d3d_semaphore_offset = param,
            0x75c => {
                let obj = ch.d3d_semaphore_obj;
                let ofs = ch.d3d_semaphore_offset;
                self.dma_write32(obj, ofs, param);
            }
            0x75d => {
                self.crtc_start = param;
                self.svga_needs_update_mode = true;
            }

            // Viewport
            0x280 if cls >= 0x0497 => {
                ch.d3d_viewport_x = param & 0xFFFF;
                ch.d3d_viewport_width = param >> 16;
            }
            0x281 if cls >= 0x0497 => {
                ch.d3d_viewport_y = param & 0xFFFF;
                ch.d3d_viewport_height = param >> 16;
            }

            // Scissor
            0x230 if cls >= 0x0497 => {
                ch.d3d_scissor_x = param & 0xFFFF;
                ch.d3d_scissor_width = param >> 16;
            }
            0x231 if cls >= 0x0497 => {
                ch.d3d_scissor_y = param & 0xFFFF;
                ch.d3d_scissor_height = param >> 16;
            }

            // Shader program
            0x239 if cls >= 0x0497 => {
                ch.d3d_shader_program = param;
                ch.d3d_shader_offset = ch.d3d_shader_program & !3;
                let location = ch.d3d_shader_program & 3;
                ch.d3d_shader_obj = match location {
                    1 => ch.d3d_a_obj,
                    2 => ch.d3d_b_obj,
                    _ => 0,
                };
            }

            // Shader control
            0x758 => ch.d3d_shader_control = param,

            // Register combiner control
            m if (m == 0x798 && cls == 0x0097) || (m == 0x23f && cls == 0x0497) => {
                ch.d3d_combiner_control = param;
                ch.d3d_combiner_control_num_stages = param & 0xf;
            }

            // Combiner final
            m if (m >= 0x0a2 && m <= 0x0a3 && cls <= 0x0097)
                || (m >= 0x23d && m <= 0x23e && cls == 0x0497) =>
            {
                let i = if cls <= 0x0097 { m - 0x0a2 } else { m - 0x23d } as usize;
                ch.d3d_combiner_final[i] = param;
            }

            // Normalize enable
            m if (m == 0x0e9 && cls <= 0x0097) || (m == 0x0df && cls >= 0x0497) => {
                ch.d3d_normalize_enable = param;
            }

            // Light enable
            m if (m == 0x0ef && cls <= 0x0097) || (m == 0x508 && cls >= 0x0497) => {
                ch.d3d_light_enable_mask = param;
            }

            // Separate specular
            m if (m == 0x0ee && cls <= 0x0097) || (m == 0x50a && cls >= 0x0497) => {
                ch.d3d_separate_specular = param & 1;
            }

            // Stencil
            m if (m == 0x0cb && cls <= 0x0097) || (m == 0x0ca && cls >= 0x0497) => {
                ch.d3d_stencil_test_enable = param;
            }

            // Blend factors (NV30+)
            0x0c5 if cls >= 0x0497 => {
                ch.d3d_blend_sfactor_rgb = param as u16;
                ch.d3d_blend_sfactor_alpha = (param >> 16) as u16;
            }
            0x0c6 if cls >= 0x0497 => {
                ch.d3d_blend_dfactor_rgb = param as u16;
                ch.d3d_blend_dfactor_alpha = (param >> 16) as u16;
            }
            0x0c8 if cls >= 0x0497 => {
                ch.d3d_blend_equation_rgb = param as u16;
                ch.d3d_blend_equation_alpha = (param >> 16) as u16;
            }

            // Transform program data
            m if (m >= 0x2c0 && m <= 0x2c3 && cls == 0x0097)
                || (m >= 0x2e0 && m <= 0x2e3 && cls >= 0x0497) =>
            {
                let i = (m & 3) as usize;
                ch.d3d_transform_program[ch.d3d_transform_program_load as usize][i] = param;
                if i == 3 {
                    ch.d3d_transform_program_load += 1;
                }
            }

            // Transform constants
            m if (m >= 0x2e0 && m <= 0x2e3 && cls == 0x0097)
                || (m >= 0x7c0 && m <= 0x7cf && cls >= 0x0497) =>
            {
                let i = (m & 3) as usize;
                ch.d3d_transform_constant[ch.d3d_transform_constant_load as usize][i] = param_float;
                if i == 3 {
                    ch.d3d_transform_constant_load += 1;
                }
            }

            // Model-view matrix
            m if (m >= 0x100 && m <= 0x11f && cls == 0x0096)
                || (m >= 0x120 && m <= 0x13f && cls >= 0x0097 && cls <= 0x0497) =>
            {
                let i = (m & 0xF) as usize;
                let mat = ((m >> 4) & 1) as usize;
                ch.d3d_model_view_matrix[mat][i] = param_float;
                // On NV35 the fixed-function matrices are also visible to the
                // vertex program as transform constants, and the guest driver
                // relies on writing only the matrix. Bochs geforce.cc
                // execute_d3d.
                if self.card_type == 0x35 {
                    ch.d3d_transform_constant[0x44 + (i >> 2)][i & 3] = param_float;
                }
            }

            // Composite matrix
            m if (m >= 0x140 && m <= 0x14f && cls == 0x0096)
                || (m >= 0x1a0 && m <= 0x1af && cls >= 0x0097 && cls <= 0x0497) =>
            {
                let i = (m & 0xF) as usize;
                ch.d3d_composite_matrix[i] = param_float;
                // Bochs geforce.cc execute_d3d — NV35 transform-constant alias.
                if self.card_type == 0x35 {
                    ch.d3d_transform_constant[0x3C + (i >> 2)][i & 3] = param_float;
                }
            }

            // Viewport offset
            m if (m >= 0x1ba && m <= 0x1bd && cls == 0x0096)
                || (m >= 0x288 && m <= 0x28b && cls >= 0x0097) =>
            {
                let i = if cls == 0x0096 { m - 0x1ba } else { m - 0x288 } as usize;
                ch.d3d_viewport_offset[i] = param_float;
            }

            // Viewport scale
            m if (m >= 0x2bc && m <= 0x2bf && cls == 0x0097)
                || (m >= 0x28c && m <= 0x28f && cls >= 0x0497) =>
            {
                let i = (m & 3) as usize;
                ch.d3d_viewport_scale[i] = param_float;
            }

            // View matrix enable
            0x0fa if cls == 0x0096 => ch.d3d_view_matrix_enable = param,

            // Transform constant load index
            m if (m == 0x7a9 && cls == 0x0097) || (m == 0x7bf && cls >= 0x0497) => {
                ch.d3d_transform_constant_load = param;
            }

            // Unhandled D3D methods - silently accept for now
            _ => {
                tracing::debug!(
                    "D3D method {:#05x} cls {:#06x} param {:#010x}",
                    method,
                    cls,
                    param
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // SVGA ports (Bochs geforce.cc svga_read / svga_write)
    // -----------------------------------------------------------------------

    /// Bochs geforce.cc `bx_geforce_c::svga_read`. `None` is its closing
    /// `VGA_READ(address, io_len)`: the core serves the access at the width
    /// the guest issued.
    fn svga_read(&mut self, cx: &mut PortCtx<'_>) -> Option<u32> {
        let port = cx.port();
        let io_len = cx.width();

        if port == PORT_VGA_ENABLE && io_len == 2 {
            let clock = cx.clock();
            let low = cx.core().read_port(PORT_VGA_ENABLE, 1, clock);
            let high = cx.core().read_port(PORT_VGA_ENABLE + 1, 1, clock);
            return Some(low | (high << 8));
        }

        if port == PORT_RMA_LOW || port == PORT_RMA_HIGH {
            return Some(self.rma_read(port, io_len));
        }

        if io_len == 2 && (port & 1) == 0 {
            let low = self.svga_read_byte(cx, port);
            let high = self.svga_read_byte(cx, port + 1);
            return Some(low | (high << 8));
        }

        if io_len != 1 {
            tracing::error!("SVGA read: io_len != 1 (port {port:#06x}, io_len {io_len})");
        }
        self.svga_read_own(port).map(u32::from)
    }

    /// One byte of a word access split at an even port — Bochs
    /// `SVGA_READ(address, 1)`: the card's answer where it has one, the
    /// core's byte otherwise.
    fn svga_read_byte(&self, cx: &mut PortCtx<'_>, port: u16) -> u32 {
        match self.svga_read_own(port) {
            Some(value) => u32::from(value),
            None => {
                let clock = cx.clock();
                cx.core().read_port(port, 1, clock)
            }
        }
    }

    /// The byte ports `svga_read`'s switch answers from the card.
    fn svga_read_own(&self, port: u16) -> Option<u8> {
        match port {
            PORT_CRTC_INDEX_MONO | PORT_CRTC_INDEX => Some(self.crtc.index),
            PORT_CRTC_DATA_MONO | PORT_CRTC_DATA
                if usize::from(self.crtc.index) > VGA_CRTC_MAX =>
            {
                Some(self.svga_read_crtc(self.crtc.index))
            }
            PORT_INPUT_STATUS_0 => {
                tracing::debug!("Input Status 0 read");
                // Monitor presence detection (DAC sensing).
                Some(0x10)
            }
            _ => None,
        }
    }

    /// Bochs geforce.cc `bx_geforce_c::svga_write`.
    fn svga_write(&mut self, cx: &mut PortCtx<'_>, value: u32) -> Written {
        let port = cx.port();
        let io_len = cx.width();

        if port == PORT_RMA_LOW || port == PORT_RMA_HIGH {
            self.rma_write(port, value, io_len);
            return Written::Done;
        }

        if io_len == 2 && (port & 1) == 0 {
            self.svga_write_byte(cx, port, value & 0xff);
            self.svga_write_byte(cx, port + 1, value >> 8);
            return Written::Done;
        }

        if io_len != 1 {
            tracing::error!("SVGA write: io_len != 1 (port {port:#06x}, io_len {io_len})");
        }
        self.svga_write_own(port, value)
    }

    /// One byte of a word access split at an even port — Bochs
    /// `SVGA_WRITE(address, value, 1)`, whose fall-through is the core's
    /// byte write.
    fn svga_write_byte(&mut self, cx: &mut PortCtx<'_>, port: u16, value: u32) {
        if self.svga_write_own(port, value) == Written::FallThrough {
            cx.core().write_port(port, value, 1);
        }
    }

    /// The byte ports `svga_write`'s switch handles.
    ///
    /// The card holds the full CRTC index, so an index above the core's range
    /// reaches the NV registers even though the core keeps only its low bits.
    /// An index write, and a data write at or below `VGA_CRTC_MAX`, also fall
    /// through so the core's own copy stays in step.
    fn svga_write_own(&mut self, port: u16, value: u32) -> Written {
        match port {
            PORT_CRTC_INDEX_MONO | PORT_CRTC_INDEX => {
                self.crtc.index = value as u8;
                Written::FallThrough
            }
            PORT_CRTC_DATA_MONO | PORT_CRTC_DATA => {
                let index = self.crtc.index;
                if CRTC_MODE_UPDATE_INDICES.contains(&index) {
                    self.svga_needs_update_mode = true;
                }
                if usize::from(index) <= VGA_CRTC_MAX {
                    self.crtc.reg[usize::from(index)] = value as u8;
                    Written::FallThrough
                } else {
                    self.svga_write_crtc(index, value as u8);
                    Written::Done
                }
            }
            _ => Written::FallThrough,
        }
    }

    /// Bochs geforce.cc `bx_geforce_c::svga_read_crtc`.
    fn svga_read_crtc(&self, index: u8) -> u8 {
        // `reg` holds indices 0..=GEFORCE_CRTC_MAX, so `get` is Bochs's bound.
        match self.crtc.reg.get(usize::from(index)) {
            Some(&value) => {
                tracing::debug!("crtc: index {index:#04x} read {value:#04x}");
                value
            }
            None => {
                tracing::error!("crtc: unknown index {index:#04x} read");
                0xff
            }
        }
    }

    /// Bochs geforce.cc `bx_geforce_c::svga_write_crtc`: the NV extension
    /// registers above the standard VGA's 0x18.
    fn svga_write_crtc(&mut self, index: u8, value: u8) {
        let slot = usize::from(index);
        if index != 0x40 || self.crtc.reg.get(slot) != Some(&value) {
            tracing::debug!("crtc: index {index:#04x} write {value:#04x}");
        }

        let mut update_cursor_addr = false;
        match index {
            0x1c => {
                // Windows 95 hangs after a reboot unless the rising edge of
                // bit 7 clears the CRTC interrupt enable.
                if (self.crtc.reg[0x1c] & 0x80) == 0 && (value & 0x80) != 0 {
                    self.crtc_intr_en = 0;
                    self.update_irq_level();
                }
            }
            0x1d | 0x1e => self.bank_base[slot - 0x1d] = u32::from(value) * 0x8000,
            0x2f..=0x31 => update_cursor_addr = true,
            0x37 | 0x3f | 0x51 => {
                let scl = (value & 0x20) != 0;
                let sda = (value & 0x10) != 0;
                if index == 0x3f {
                    self.ddc.write(scl, sda);
                    self.crtc.reg[0x3e] = self.ddc.read() & 0x0c;
                } else {
                    self.crtc.reg[slot - 1] = (u8::from(sda) << 3) | (u8::from(scl) << 2);
                }
            }
            // Paired with 0x57, this windows 16 bytes of head-specific state.
            // Writes are dropped, so reads return zero until something visible
            // depends on it.
            0x58 => return,
            _ => {}
        }

        match self.crtc.reg.get_mut(slot) {
            Some(reg) => *reg = value,
            None => tracing::error!("crtc: unknown index {index:#04x} write"),
        }

        if update_cursor_addr {
            self.hw_cursor.enabled = (self.crtc.reg[0x31] & 0x01) != 0
                || (self.crtc_cursor_config & 0x0000_0001) != 0;
            self.hw_cursor.vram = (self.crtc.reg[0x30] & 0x80) != 0
                || (self.crtc_cursor_config & 0x0000_0100) != 0
                || self.card_type >= 0x40;
            let offset = ((u32::from(self.crtc.reg[0x31]) >> 2) << 11)
                | (u32::from(self.crtc.reg[0x30] & 0x7f) << 17)
                | (u32::from(self.crtc.reg[0x2f]) << 24);
            self.hw_cursor.offset = offset.wrapping_add(self.crtc_cursor_offset);
        }
    }

    /// Where `address` lands for an RMA data transfer, or `None` when it is
    /// past the end of its target.
    fn rma_target(&self, address: u32) -> Option<RmaTarget> {
        if (address & 0x8000_0000) != 0 {
            let offset = address & !0x8000_0000;
            (offset < self.memsize).then_some(RmaTarget::Vram(offset))
        } else {
            (address < GEFORCE_PNPMMIO_SIZE).then_some(RmaTarget::Register(address))
        }
    }

    fn rma_load(&self, target: RmaTarget) -> u32 {
        match target {
            RmaTarget::Register(offset) => self.register_read32(offset),
            RmaTarget::Vram(offset) => self.vram_read32(offset),
        }
    }

    fn rma_store(&mut self, target: RmaTarget, value: u32) {
        match target {
            RmaTarget::Register(offset) => self.register_write32(offset, value),
            RmaTarget::Vram(offset) => self.vram_write32(offset, value),
        }
    }

    /// The Real Mode Access window at 0x3D0/0x3D2 — the read half of Bochs
    /// geforce.cc `svga_read`'s `RMA_ACCESS` case. CRTC 0x38 bit 0 opens the
    /// window and bits 7:1 select what it reads: 1 the address, 2 the data.
    fn rma_read(&self, port: u16, io_len: u8) -> u32 {
        if io_len == 1 {
            tracing::error!("port 0x3d0 access with io_len = 1");
            return 0;
        }
        let crtc38 = self.crtc.reg[0x38];
        if (crtc38 & 1) == 0 {
            tracing::error!("port 0x3d0 access is disabled");
            return 0;
        }
        match crtc38 >> 1 {
            1 => {
                if port == PORT_RMA_LOW {
                    self.rma_addr
                } else {
                    self.rma_addr >> 16
                }
            }
            2 => match self.rma_target(self.rma_addr) {
                Some(target) => {
                    let value = self.rma_load(target);
                    tracing::debug!("rma: read from {:#010x} value {value:#010x}", self.rma_addr);
                    if port == PORT_RMA_LOW {
                        value
                    } else {
                        value >> 16
                    }
                }
                None => {
                    tracing::error!("rma: oob read from {:#010x} ignored", self.rma_addr);
                    0xFFFF_FFFF
                }
            },
            3 => {
                tracing::error!("rma: read index 3");
                0
            }
            rma_index => {
                tracing::error!("rma: read unknown index {rma_index}");
                0
            }
        }
    }

    /// The write half of [`Self::rma_read`]: index 1 sets the address, index 3
    /// writes the data. A 16-bit write replaces the half the port names.
    fn rma_write(&mut self, port: u16, value: u32, io_len: u8) {
        if io_len == 1 {
            tracing::debug!("port 0x3d0 access with io_len = 1");
            return;
        }
        let crtc38 = self.crtc.reg[0x38];
        if (crtc38 & 1) == 0 {
            tracing::error!("port 0x3d0 access is disabled");
            return;
        }
        match crtc38 >> 1 {
            1 => {
                if port == PORT_RMA_LOW {
                    if io_len == 2 {
                        self.rma_addr = (self.rma_addr & 0xFFFF_0000) | value;
                    } else {
                        self.rma_addr = value;
                    }
                } else {
                    self.rma_addr = (self.rma_addr & 0x0000_FFFF) | (value << 16);
                }
            }
            2 => tracing::error!("rma: write index 2"),
            3 => {
                // The data transfer is dword-aligned — Bochs's fix for the
                // 6800 GT BIOS.
                match self.rma_target(self.rma_addr & !3) {
                    Some(target) => {
                        let merged = if port == PORT_RMA_LOW {
                            if io_len == 2 {
                                (self.rma_load(target) & 0xFFFF_0000) | value
                            } else {
                                value
                            }
                        } else {
                            (self.rma_load(target) & 0x0000_FFFF) | (value << 16)
                        };
                        self.rma_store(target, merged);
                        tracing::debug!("rma: write to {:#010x} value {merged:#010x}", self.rma_addr);
                    }
                    None => {
                        tracing::error!("rma: oob write to {:#010x} ignored", self.rma_addr);
                    }
                }
            }
            rma_index => tracing::error!("rma: write unknown index {rma_index}"),
        }
    }

    fn register_read8(&self, address: u32) -> u8 {
        match address {
            a if a >= 0x1800 && a < 0x1900 => self.pci_conf[(a - 0x1800) as usize],
            a if a >= 0x700000 && a < 0x800000 => self.vram_read8((a - 0x700000) ^ self.ramin_flip),
            _ => self.register_read32(address) as u8,
        }
    }

    fn register_write8(&mut self, address: u32, value: u8) {
        match address {
            a if a >= 0x700000 && a < 0x800000 => {
                let addr = (a - 0x700000) ^ self.ramin_flip;
                self.vram_write8(addr, value);
            }
            _ => {
                let current = self.register_read32(address);
                self.register_write32(address, (current & !0xFF) | value as u32);
            }
        }
    }

    /// Read VRAM at an address within BAR1.
    pub fn vram_bar_read(&self, offset: u32) -> u8 {
        self.memory[(offset & self.memsize_mask) as usize]
    }

    /// Write VRAM at an address within BAR1.
    pub fn vram_bar_write(&mut self, offset: u32, value: u8) {
        let addr = (offset & self.memsize_mask) as usize;
        self.memory[addr] = value;
    }

    /// Set the current time in nanoseconds (called by emulator timer system).
    pub fn set_time_nsec(&mut self, nsec: u64) {
        self.time_nsec = nsec;
    }

    /// Vertical timer callback - handle vsync interrupts.
    pub fn vertical_timer(&mut self) {
        self.crtc_intr |= 0x0000_0001;
        self.update_irq_level();
        if self.fifo_wait_acquire {
            self.fifo_wait_acquire = false;
            self.update_fifo_wait();
            self.fifo_process_all();
        }
    }
}

/// The GeForce as a display adapter — the seam's second implementor.
///
/// Upstream `bx_geforce_c` derives from `bx_vgacore_c`, not from `bx_vga_c`, so
/// a GeForce is a standard VGA plus an NV chip and has no Bochs VBE extension
/// at all. That is what this composition says, and it is why the VBE state
/// moved out of the core: a card that never had DISPI registers should not
/// carry them.
///
/// The NV scanout is deliberately not wired, so `vga_refresh` always falls
/// through to the core's legacy render — also when a guest has set CRTC 0x28
/// and turned the extended scanout on. That matches Bochs geforce.cc
/// `bx_geforce_c::update()` only while CRTC 0x28 is 0 below bit 7 (bit 7 marks
/// slaved mode and is masked off), where upstream too calls
/// `bx_vgacore_c::update()`; otherwise upstream draws the NV framebuffer and
/// this card still draws the legacy planes. Wiring the NV path
/// is its own work; this proves the seam carries a second card whose state and
/// windows are nothing like the first one's.
impl VgaExtension for BxGeForceC {
    /// What the CORE must allocate, which is not the same as what the card has.
    ///
    /// The core renders only the legacy modes for this card — the NV scanout is
    /// not wired — so it needs planar memory and nothing more. The chip's own
    /// framebuffer, 64 MB on a GeForce2 up to 256 MB on a 6800, stays where the
    /// chip keeps it; reporting that here would allocate a second copy of it.
    /// When the NV scanout is wired the two become one buffer, which is what
    /// the core-owns-VRAM rule is for.
    fn vga_vram_bytes(&self) -> u32 {
        crate::display::vga::VgaCore::LEGACY_VRAM_BYTES as u32
    }

    fn vga_max_resolution(&self) -> (u32, u32) {
        // The NV CRTC drives its own scanout, so the ceiling is the card's, not
        // a DISPI register's.
        (self.svga_xres.max(640), self.svga_yres.max(480))
    }

    /// Video memory reads land in the NV framebuffer, which the chip addresses
    /// linearly rather than through the planar core.
    fn vga_mem_read(&mut self, cx: &mut MemCtx<'_>) -> Option<u8> {
        match cx.window() {
            // The legacy aperture stays the core's: a GeForce in a text mode is
            // a standard VGA, latches and plane masks included.
            VgaWindow::Legacy => None,
            VgaWindow::Lfb => {
                let at = cx.offset().get() as u32 & self.memsize_mask;
                Some(self.vram_read8(at))
            }
            VgaWindow::Registers => None,
        }
    }

    fn vga_mem_write(&mut self, cx: &mut MemCtx<'_>, value: u8) -> Written {
        match cx.window() {
            VgaWindow::Lfb => {
                let at = cx.offset().get() as u32 & self.memsize_mask;
                self.vram_write8(at, value);
                Written::Done
            }
            _ => Written::FallThrough,
        }
    }

    /// BAR0, the NV register file — Bochs geforce.cc
    /// `geforce_mem_read_handler`. The offset is taken modulo the window, and
    /// a read is 1, 2 or 4 bytes wide, a 2-byte read being the low half of the
    /// 32-bit register. Registers format to the access width, which is why the
    /// window is offered whole rather than a byte at a time.
    fn vga_regs_read(&mut self, cx: &mut MemCtx<'_>, len: u32, data: &mut [u8]) -> Written {
        if cx.window() != VgaWindow::Registers {
            return Written::FallThrough;
        }
        let offset = bar0_offset(cx.offset());
        match len {
            1 => fill_access(data, &[self.register_read8(offset)]),
            2 => fill_access(data, &(self.register_read32(offset) as u16).to_le_bytes()),
            4 => fill_access(data, &self.register_read32(offset).to_le_bytes()),
            _ => tracing::error!("MMIO read len {len}"),
        }
        Written::Done
    }

    /// The write half of [`Self::vga_regs_read`] — Bochs geforce.cc
    /// `geforce_mem_write_handler`: 1, 4 or 8 bytes, an 8-byte write being two
    /// register writes, the low dword first.
    fn vga_regs_write(&mut self, cx: &mut MemCtx<'_>, len: u32, data: &[u8]) -> Written {
        if cx.window() != VgaWindow::Registers {
            return Written::FallThrough;
        }
        let offset = bar0_offset(cx.offset());
        match len {
            1 => self.register_write8(offset, u8::from_le_bytes(access_bytes(data))),
            4 => self.register_write32(offset, u32::from_le_bytes(access_bytes(data))),
            8 => {
                let value = u64::from_le_bytes(access_bytes(data));
                self.register_write32(offset, value as u32);
                self.register_write32(offset + 4, (value >> 32) as u32);
            }
            _ => tracing::error!("MMIO write len {len}"),
        }
        Written::Done
    }

    /// The NV CRTC registers, the RMA window and Input Status 0 — Bochs
    /// geforce.cc `svga_read`, which `init_vga_extension` installs over the
    /// standard VGA's port handlers.
    fn vga_pio_read(&mut self, cx: &mut PortCtx<'_>) -> Option<u32> {
        self.svga_read(cx)
    }

    /// Bochs geforce.cc `svga_write`.
    fn vga_pio_write(&mut self, cx: &mut PortCtx<'_>, value: u32) -> Written {
        self.svga_write(cx, value)
    }

    /// Bochs `bx_geforce_c::reset` resets the NV chip; the core has already
    /// reset itself by the time this runs.
    fn vga_reset(&mut self, _cx: &mut ResetCtx<'_>) {
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{
        DeviceCtx, IoLen, IrqLine, IrqSink, MmioDevice, PioDevice, TimerKey, TimerService,
    };
    use crate::display::card::VgaCard;
    use rusty_box_core::time::{ClockHz, VmClock, VmInstant};

    const CLOCK: VmClock = VmClock::new(VmInstant::ZERO, ClockHz::BOCHS_DEFAULT);

    /// The interrupt and timer capabilities an access is handed. The NV
    /// chip's interrupt line reaches no controller, so nothing here is read.
    struct Unwired;

    impl IrqSink for Unwired {
        fn set_level(&mut self, _line: IrqLine, _level: bool) {}
        fn level(&self, _line: IrqLine) -> bool {
            false
        }
    }

    impl TimerService for Unwired {
        fn arm_oneshot_usec(&mut self, _key: TimerKey, _delay_usec: u64) {}
        fn arm_periodic_usec(&mut self, _key: TimerKey, _period_usec: u64) {}
        fn arm_oneshot_ticks(&mut self, _key: TimerKey, _delay_ticks: u64) {}
        fn cancel(&mut self, _key: TimerKey) {}
    }

    /// A GeForce3 driven the way a guest drives one: through its ports and its
    /// BAR0 window.
    struct Guest {
        card: VgaCard<BxGeForceC>,
        irq: Unwired,
        timers: Unwired,
    }

    impl Guest {
        fn new() -> Self {
            Self {
                card: VgaCard::with_extension(BxGeForceC::new(GeForceModel::GeForce3)),
                irq: Unwired,
                timers: Unwired,
            }
        }

        fn port_in(&mut self, port: u16, len: IoLen) -> u32 {
            let mut ctx = DeviceCtx {
                clock: CLOCK,
                irq: &mut self.irq,
                timers: &mut self.timers,
            };
            PioDevice::pio_read(&mut self.card, port, len, &mut ctx)
        }

        fn port_out(&mut self, port: u16, value: u32, len: IoLen) {
            let mut ctx = DeviceCtx {
                clock: CLOCK,
                irq: &mut self.irq,
                timers: &mut self.timers,
            };
            PioDevice::pio_write(&mut self.card, port, value, len, &mut ctx);
        }

        fn crtc_write(&mut self, index: u8, value: u8) {
            self.port_out(PORT_CRTC_INDEX, u32::from(index), IoLen::Byte);
            self.port_out(PORT_CRTC_DATA, u32::from(value), IoLen::Byte);
        }

        fn crtc_read(&mut self, index: u8) -> u32 {
            self.port_out(PORT_CRTC_INDEX, u32::from(index), IoLen::Byte);
            self.port_in(PORT_CRTC_DATA, IoLen::Byte)
        }

        fn bar0_read(&mut self, offset: u32, len: u32) -> u64 {
            let mut data = [0u8; 8];
            let mut ctx = DeviceCtx {
                clock: CLOCK,
                irq: &mut self.irq,
                timers: &mut self.timers,
            };
            MmioDevice::mmio_read(
                &mut self.card,
                VgaWindow::Registers.id(),
                WindowOffset(u64::from(offset)),
                len,
                &mut data[..len as usize],
                &mut ctx,
            );
            u64::from_le_bytes(data)
        }

        fn bar0_write(&mut self, offset: u32, value: u64, len: u32) {
            let data = value.to_le_bytes();
            let mut ctx = DeviceCtx {
                clock: CLOCK,
                irq: &mut self.irq,
                timers: &mut self.timers,
            };
            MmioDevice::mmio_write(
                &mut self.card,
                VgaWindow::Registers.id(),
                WindowOffset(u64::from(offset)),
                len,
                &data[..len as usize],
                &mut ctx,
            );
        }

        /// Open the RMA window at `rma_index` — CRTC 0x38, bit 0 the enable.
        fn rma_select(&mut self, rma_index: u8) {
            self.crtc_write(0x38, (rma_index << 1) | 1);
        }

        /// Drive the DDC clock and data lines through CRTC 0x3F.
        fn ddc_drive(&mut self, scl: bool, sda: bool) {
            self.crtc_write(0x3F, (u8::from(scl) << 5) | (u8::from(sda) << 4));
        }

        /// The data line as the monitor holds it, sampled through CRTC 0x3E.
        fn ddc_monitor_sda(&mut self) -> bool {
            (self.crtc_read(0x3E) & 0x08) != 0
        }

        /// Clock one host bit out: data while the clock is low, then a pulse.
        fn ddc_send_bit(&mut self, bit: bool) {
            self.ddc_drive(false, bit);
            self.ddc_drive(true, bit);
            self.ddc_drive(false, bit);
        }

        /// START, then the monitor's address 0x50 with the read bit — which
        /// leaves the monitor holding data low for its acknowledge.
        fn ddc_address_monitor_for_read(&mut self) {
            self.ddc_drive(true, true);
            self.ddc_drive(true, false);
            self.ddc_drive(false, false);
            for shift in (0..7).rev() {
                self.ddc_send_bit(((0x50 >> shift) & 1) != 0);
            }
            self.ddc_send_bit(true);
        }

        fn ddc_receive_byte(&mut self) -> u8 {
            let mut byte = 0u8;
            for _ in 0..8 {
                self.ddc_drive(true, true);
                byte = (byte << 1) | u8::from(self.ddc_monitor_sda());
                self.ddc_drive(false, true);
            }
            byte
        }
    }

    /// An index past the standard VGA's belongs to the card alone. The core
    /// keeps six index bits, so index 0x41 reaching it would land on CRTC 0x01,
    /// the horizontal display end of the mode on screen.
    #[test]
    fn an_extended_crtc_write_reaches_the_card_and_not_the_core() {
        let mut guest = Guest::new();
        guest.crtc_write(0x01, 0x4F);

        guest.crtc_write(0x41, 0x5A);

        assert_eq!(guest.crtc_read(0x41), 0x5A, "the card stores and answers index 0x41");
        assert_eq!(guest.crtc_read(0x01), 0x4F, "and the core's CRTC 0x01 is untouched");
    }

    /// At or below 0x18 the card keeps a copy and the core still performs the
    /// write — Bochs `svga_write` stores `crtc.reg` and then calls `VGA_WRITE`.
    #[test]
    fn a_standard_crtc_write_reaches_both_the_card_and_the_core() {
        let mut guest = Guest::new();

        guest.crtc_write(0x13, 0x28);

        assert_eq!(guest.crtc_read(0x13), 0x28, "the core answers the read");
        assert_eq!(
            guest.card.ext.crtc.reg[0x13], 0x28,
            "the card's copy, which its extended modes are computed from"
        );
        assert!(guest.card.ext.svga_needs_update_mode, "CRTC 0x13 is a mode register");
    }

    #[test]
    fn the_crtc_index_reads_back_in_full() {
        let mut guest = Guest::new();

        guest.port_out(PORT_CRTC_INDEX, 0x41, IoLen::Byte);

        assert_eq!(guest.port_in(PORT_CRTC_INDEX, IoLen::Byte), 0x41);
        assert_eq!(
            guest.port_in(PORT_CRTC_INDEX_MONO, IoLen::Byte),
            0x41,
            "the mono index port answers from the same register"
        );
    }

    /// Bochs answers Input Status 0 with the DAC-sensing bit set: a monitor is
    /// present.
    #[test]
    fn input_status_0_reports_a_monitor() {
        let mut guest = Guest::new();
        assert_eq!(guest.port_in(PORT_INPUT_STATUS_0, IoLen::Byte), 0x10);
    }

    /// A word at the index port carries the index in its low byte and the
    /// register in its high byte, and the card resolves both halves.
    #[test]
    fn a_word_crtc_access_splits_into_index_and_data() {
        let mut guest = Guest::new();
        guest.crtc_write(0x01, 0x4F);

        guest.port_out(PORT_CRTC_INDEX, 0x5A41, IoLen::Word);

        assert_eq!(guest.port_in(PORT_CRTC_INDEX, IoLen::Word), 0x5A41);
        assert_eq!(
            guest.crtc_read(0x01),
            0x4F,
            "the data byte went to the card's 0x41, not to CRTC 0x01"
        );
    }

    /// The RMA window is how an NV video BIOS reaches BAR0 and VRAM through
    /// I/O ports: CRTC 0x38 selects the address or the data, and 0x3D0/0x3D2
    /// carry it.
    #[test]
    fn rma_reaches_the_register_file_and_vram() {
        let mut guest = Guest::new();

        // PFIFO RAMHT, through the register file.
        guest.rma_select(1);
        guest.port_out(PORT_RMA_LOW, 0x0000_2210, IoLen::Dword);
        assert_eq!(guest.port_in(PORT_RMA_LOW, IoLen::Dword), 0x0000_2210);
        guest.rma_select(3);
        guest.port_out(PORT_RMA_LOW, 0xCAFE_BABE, IoLen::Dword);
        assert_eq!(guest.bar0_read(0x2210, 4), 0xCAFE_BABE, "the register BAR0 shows");

        // A word at 0x3D2 replaces the high half only.
        guest.port_out(PORT_RMA_HIGH, 0x1234, IoLen::Word);
        guest.rma_select(2);
        assert_eq!(guest.port_in(PORT_RMA_LOW, IoLen::Dword), 0x1234_BABE);
        assert_eq!(guest.port_in(PORT_RMA_HIGH, IoLen::Word), 0x1234);

        // Address bit 31 selects VRAM; each address half is set by its port.
        guest.rma_select(1);
        guest.port_out(PORT_RMA_LOW, 0x1000, IoLen::Word);
        guest.port_out(PORT_RMA_HIGH, 0x8000, IoLen::Word);
        guest.rma_select(3);
        guest.port_out(PORT_RMA_LOW, 0x0102_0304, IoLen::Dword);
        guest.rma_select(2);
        assert_eq!(guest.port_in(PORT_RMA_LOW, IoLen::Dword), 0x0102_0304);
        assert_eq!(guest.card.ext.vram_read32(0x1000), 0x0102_0304, "and it is VRAM");
    }

    #[test]
    fn rma_is_closed_until_crtc_0x38_opens_it() {
        let mut guest = Guest::new();

        guest.port_out(PORT_RMA_LOW, 0x0000_2210, IoLen::Dword);
        assert_eq!(guest.port_in(PORT_RMA_LOW, IoLen::Dword), 0, "a closed window reads zero");

        guest.rma_select(1);
        assert_eq!(
            guest.port_in(PORT_RMA_LOW, IoLen::Dword),
            0,
            "the address written while it was closed was dropped"
        );
        assert_eq!(guest.port_in(PORT_RMA_LOW, IoLen::Byte), 0, "a byte access reads zero");
    }

    /// The window bounds a transfer by its first byte only, so a read at the
    /// last two bytes of VRAM straddles the end. The bytes that exist come
    /// back and the access completes.
    #[test]
    fn an_rma_read_straddling_the_end_of_vram_returns_the_bytes_that_exist() {
        let mut guest = Guest::new();
        let last_dword = guest.card.ext.memsize - 4;
        guest.rma_select(1);
        guest.port_out(PORT_RMA_LOW, 0x8000_0000 | last_dword, IoLen::Dword);
        guest.rma_select(3);
        guest.port_out(PORT_RMA_LOW, 0x4433_2211, IoLen::Dword);

        guest.rma_select(1);
        guest.port_out(PORT_RMA_LOW, 0x8000_0000 | (last_dword + 2), IoLen::Dword);
        guest.rma_select(2);

        assert_eq!(guest.port_in(PORT_RMA_LOW, IoLen::Dword), 0x0000_4433);
    }

    #[test]
    fn bar0_reads_are_one_two_or_four_bytes_wide() {
        let mut guest = Guest::new();
        guest.bar0_write(0x2210, 0x1234_5678, 4);

        assert_eq!(guest.bar0_read(0x2210, 1), 0x78);
        assert_eq!(guest.bar0_read(0x2210, 2), 0x5678, "the low half of the register");
        assert_eq!(guest.bar0_read(0x2210, 4), 0x1234_5678);
    }

    #[test]
    fn an_eight_byte_bar0_write_stores_both_dwords() {
        let mut guest = Guest::new();

        guest.bar0_write(0x2210, 0x1111_2222_3333_4444, 8);

        assert_eq!(guest.bar0_read(0x2210, 4), 0x3333_4444, "PFIFO RAMHT takes the low dword");
        assert_eq!(guest.bar0_read(0x2214, 4), 0x1111_2222, "PFIFO RAMFC the high one");

        guest.bar0_write(0x2210, 0xAB, 1);
        assert_eq!(guest.bar0_read(0x2210, 4), 0x3333_44AB, "a byte write replaces one byte");
    }

    /// A dword read at 0x18FE straddles the end of the 256-byte PCI config
    /// space mirrored in BAR0.
    #[test]
    fn a_dword_read_at_the_end_of_pci_config_space_completes() {
        let mut guest = Guest::new();
        guest.bar0_write(0x18FC, 0x4433_2211, 4);

        assert_eq!(guest.bar0_read(0x18FE, 4), 0x0000_4433);
    }

    /// The NV BIOS reads the monitor's EDID by bit-banging I2C through CRTC
    /// 0x3F and sampling the lines back through 0x3E.
    #[test]
    fn the_edid_reads_through_crtc_0x3f_and_0x3e() {
        let mut guest = Guest::new();

        guest.ddc_drive(true, true);
        assert_eq!(guest.crtc_read(0x3E), 0x0C, "clock and data both high at rest");

        guest.ddc_address_monitor_for_read();
        guest.ddc_drive(false, true);
        guest.ddc_drive(true, true);
        assert!(!guest.ddc_monitor_sda(), "the monitor acknowledges address 0x50");
        guest.ddc_drive(false, true);

        assert_eq!(guest.ddc_receive_byte(), 0x00, "EDID header byte 0");
        guest.ddc_send_bit(false);
        guest.ddc_drive(false, true);
        assert_eq!(guest.ddc_receive_byte(), 0xFF, "EDID header byte 1");
    }

    /// CRTC 0x37 and 0x51 report the lines they are given in the register
    /// one below: data in bit 3, clock in bit 2.
    #[test]
    fn crtc_0x37_and_0x51_report_their_lines_one_index_below() {
        let mut guest = Guest::new();

        guest.crtc_write(0x37, 0x20);
        assert_eq!(guest.crtc_read(0x36), 0x04, "clock high, data low");

        guest.crtc_write(0x51, 0x10);
        assert_eq!(guest.crtc_read(0x50), 0x08, "clock low, data high");
    }

    #[test]
    fn crtc_0x1c_bit_7_rising_clears_the_crtc_interrupt_enable() {
        let mut guest = Guest::new();
        guest.bar0_write(0x60_0140, 1, 4);

        guest.crtc_write(0x1C, 0x80);

        assert_eq!(guest.bar0_read(0x60_0140, 4), 0, "PCRTC_INTR_EN cleared on the edge");
        assert_eq!(guest.crtc_read(0x1C), 0x80);

        guest.bar0_write(0x60_0140, 1, 4);
        guest.crtc_write(0x1C, 0x80);
        assert_eq!(guest.bar0_read(0x60_0140, 4), 1, "bit 7 already set is no edge");
    }

    /// CRTC 0x2F-0x31 place the hardware cursor image and enable it — the
    /// state the NV scanout draws the cursor from.
    #[test]
    fn crtc_0x2f_to_0x31_place_and_enable_the_hardware_cursor() {
        let mut guest = Guest::new();
        guest.crtc_write(0x30, 0x81);
        guest.crtc_write(0x2F, 0x02);
        assert!(!guest.card.ext.hw_cursor.enabled);

        guest.crtc_write(0x31, 0x05);

        let cursor = &guest.card.ext.hw_cursor;
        assert!(cursor.enabled, "CRTC 0x31 bit 0");
        assert!(cursor.vram, "CRTC 0x30 bit 7");
        assert_eq!(cursor.offset, (0x02 << 24) | (0x01 << 17) | (0x01 << 11));
    }

    #[test]
    fn crtc_0x1d_and_0x1e_select_32k_banks() {
        let mut guest = Guest::new();

        guest.crtc_write(0x1D, 3);
        guest.crtc_write(0x1E, 1);

        assert_eq!(guest.card.ext.bank_base, [3 * 0x8000, 0x8000]);
    }

    #[test]
    fn crtc_0x58_drops_writes_that_its_pair_register_keeps() {
        let mut guest = Guest::new();

        guest.crtc_write(0x57, 0x12);
        guest.crtc_write(0x58, 0x34);

        assert_eq!(guest.crtc_read(0x57), 0x12);
        assert_eq!(guest.crtc_read(0x58), 0x00);
    }

    #[test]
    fn a_crtc_index_past_the_register_file_reads_all_ones() {
        let mut guest = Guest::new();

        guest.crtc_write(0xF5, 0x12);

        assert_eq!(guest.crtc_read(0xF5), 0xFF);
        assert_eq!(guest.crtc_read(0xF0), 0x00, "0xF0 is the last register in the file");
    }

    /// Bochs `bx_geforce_c::reset` restores the power-on straps, clears the
    /// ROM-shadow enable in PCI config 0x50, and restarts the DDC channel.
    #[test]
    fn reset_restores_the_straps_the_rom_shadow_enable_and_the_ddc_channel() {
        let mut guest = Guest::new();
        let power_on_straps = guest.bar0_read(0x10_1000, 4);
        guest.bar0_write(0x10_1000, 0x8000_0001, 4);
        guest.bar0_write(0x1850, 0x01, 1);
        assert_eq!(guest.bar0_read(0x10_1000, 4), 0x8000_0001);
        assert_eq!(guest.bar0_read(0x1850, 1), 0x01);
        guest.ddc_address_monitor_for_read();

        guest.card.reset();

        assert_eq!(guest.bar0_read(0x10_1000, 4), power_on_straps);
        assert_eq!(guest.bar0_read(0x1850, 1), 0x00);
        // A monitor still in its acknowledge would hold data low here.
        guest.ddc_drive(true, true);
        assert_eq!(guest.crtc_read(0x3E), 0x0C, "the channel is back at rest");
    }
}
