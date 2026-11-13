//! D3D11 디바이스와 스왑체인.
//!
//! 디바이스는 하나만 만들어 FFmpeg 디코더와 렌더러가 함께 씁니다. 즉시 컨텍스트는
//! 양쪽이 같이 쓰므로, 렌더러도 FFmpeg가 쓰는 잠금(`AVD3D11VADeviceContext.lock`)을
//! 잡고 부릅니다.

use std::ffi::c_void;

use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::Interface;

use crate::media::ffi::{self, AVD3D11VADeviceContext, BufferRef, sys};

pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub video_device: ID3D11VideoDevice,
    pub video_context: ID3D11VideoContext,
    /// FFmpeg의 D3D11VA 하드웨어 디바이스 컨텍스트. 디코더마다 참조를 하나씩 줍니다.
    pub hw: BufferRef,
    lock: unsafe extern "C" fn(*mut c_void),
    unlock: unsafe extern "C" fn(*mut c_void),
    lock_ctx: *mut c_void,
    pub adapter_name: String,
}

// COM 객체는 다중 스레드 보호를 켠 디바이스의 것이고, 즉시 컨텍스트는 잠금 아래에서만 씁니다.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

pub struct GpuLock<'a>(&'a Gpu);

impl Drop for GpuLock<'_> {
    fn drop(&mut self) {
        unsafe { (self.0.unlock)(self.0.lock_ctx) }
    }
}

impl Gpu {
    pub fn new() -> Result<Self, String> {
        crate::shared::timing("  Gpu::new 시작");
        let flags = D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                flags,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11 디바이스를 만들 수 없습니다: {e}"))?;
        }
        crate::shared::timing("  D3D11CreateDevice");
        let device: ID3D11Device = device.ok_or("D3D11 디바이스 없음")?;
        let context: ID3D11DeviceContext = context.ok_or("D3D11 컨텍스트 없음")?;

        // 디코더 스레드와 렌더 스레드가 같은 디바이스를 씁니다.
        if let Ok(mt) = context.cast::<ID3D11Multithread>() {
            unsafe {
                let _ = mt.SetMultithreadProtected(true);
            }
        }
        let video_device: ID3D11VideoDevice = device
            .cast()
            .map_err(|e| format!("영상 디바이스를 쓸 수 없습니다: {e}"))?;
        let video_context: ID3D11VideoContext = context
            .cast()
            .map_err(|e| format!("영상 컨텍스트를 쓸 수 없습니다: {e}"))?;

        let adapter_name = adapter_name(&device);
        crate::shared::timing("  어댑터 이름");

        unsafe {
            let hw = sys::av_hwdevice_ctx_alloc(sys::AVHWDeviceType::D3D11VA);
            if hw.is_null() {
                return Err("FFmpeg 하드웨어 컨텍스트를 만들 수 없습니다".into());
            }
            let hw = BufferRef(hw);
            let devctx = (*hw.0).data as *mut sys::AVHWDeviceContext;
            let d3d = (*devctx).hwctx as *mut AVD3D11VADeviceContext;
            // FFmpeg가 해제할 때 Release하므로 참조를 하나 넘깁니다.
            (*d3d).device = device.clone().into_raw();
            ffi::check(sys::av_hwdevice_ctx_init(hw.0), "av_hwdevice_ctx_init")?;
            crate::shared::timing("  av_hwdevice_ctx_init");
            let lock = (*d3d).lock.ok_or("FFmpeg 잠금 없음")?;
            let unlock = (*d3d).unlock.ok_or("FFmpeg 잠금 없음")?;
            let lock_ctx = (*d3d).lock_ctx;
            Ok(Self {
                device,
                context,
                video_device,
                video_context,
                hw,
                lock,
                unlock,
                lock_ctx,
                adapter_name,
            })
        }
    }

    /// 즉시 컨텍스트를 쓰기 전에 잡습니다. FFmpeg 디코더와 직렬화됩니다.
    pub fn lock(&self) -> GpuLock<'_> {
        unsafe { (self.lock)(self.lock_ctx) };
        GpuLock(self)
    }

    /// GPU가 이 디코더 프로파일을 지원하는지.
    pub fn supports_profile(&self, profile: &windows::core::GUID) -> bool {
        unsafe {
            let n = self.video_device.GetVideoDecoderProfileCount();
            (0..n)
                .any(|i| self.video_device.GetVideoDecoderProfile(i).ok().as_ref() == Some(profile))
        }
    }

    /// 디바이스를 잃었는지. 잃었다면 이유를 돌려줍니다.
    pub fn removed_reason(&self) -> Option<String> {
        let hr = unsafe { self.device.GetDeviceRemovedReason() };
        hr.err().map(|e| e.to_string())
    }
}

fn adapter_name(device: &ID3D11Device) -> String {
    unsafe {
        let Ok(dxgi) = device.cast::<IDXGIDevice>() else {
            return String::new();
        };
        let Ok(adapter) = dxgi.GetAdapter() else {
            return String::new();
        };
        let Ok(desc) = adapter.GetDesc() else {
            return String::new();
        };
        let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(128);
        String::from_utf16_lossy(&desc.Description[..len])
    }
}

/// 플립 모델 스왑체인. 수직 동기에 맞춰 한 장씩 내보냅니다.
pub struct Swap {
    pub chain: IDXGISwapChain2,
    pub waitable: HANDLE,
    pub width: u32,
    pub height: u32,
}

impl Swap {
    pub fn new(gpu: &Gpu, hwnd: HWND, width: u32, height: u32) -> Result<Self, String> {
        unsafe {
            let dxgi: IDXGIDevice = gpu.device.cast().map_err(|e| e.to_string())?;
            let adapter = dxgi.GetAdapter().map_err(|e| e.to_string())?;
            let factory: IDXGIFactory2 = adapter.GetParent().map_err(|e| e.to_string())?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width.max(1),
                Height: height.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 3,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
                ..Default::default()
            };
            let chain = {
                let _l = gpu.lock();
                factory
                    .CreateSwapChainForHwnd(&gpu.device, hwnd, &desc, None, None)
                    .map_err(|e| format!("스왑체인을 만들 수 없습니다: {e}"))?
            };
            // Alt+Enter 전체화면은 창에서 직접 처리합니다.
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            let chain: IDXGISwapChain2 = chain.cast().map_err(|e| e.to_string())?;
            let _ = chain.SetMaximumFrameLatency(2);
            let waitable = chain.GetFrameLatencyWaitableObject();
            Ok(Self {
                chain,
                waitable,
                width: width.max(1),
                height: height.max(1),
            })
        }
    }

    /// 백버퍼 크기를 바꿉니다. 백버퍼를 가리키는 모든 뷰를 먼저 놓아야 합니다.
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) -> Result<(), String> {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return Ok(());
        }
        let _l = gpu.lock();
        unsafe {
            gpu.context.ClearState();
            gpu.context.Flush();
            self.chain
                .ResizeBuffers(
                    0,
                    width,
                    height,
                    DXGI_FORMAT_UNKNOWN,
                    DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
                )
                .map_err(|e| format!("ResizeBuffers: {e}"))?;
        }
        self.width = width;
        self.height = height;
        Ok(())
    }

    pub fn back_buffer(&self) -> Result<ID3D11Texture2D, String> {
        unsafe { self.chain.GetBuffer(0).map_err(|e| e.to_string()) }
    }
}
