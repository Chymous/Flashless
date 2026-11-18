//! Captures the League window with Windows.Graphics.Capture (the API behind OBS "Window Capture").
//! Frames stay on the GPU: the chat area is cut out there (CopySubresourceRegion into a small staging
//! texture) and only that small crop is read back to the CPU, instead of the whole 1440p frame.

use windows::core::{factory, Interface};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

/// BGRA32 pixels, top-down rows, stride = w * 4.
#[derive(Clone)]
pub struct Frame {
    pub px: Vec<u8>,
    pub w: usize,
    pub h: usize,
}

/// Part of the window as fractions: left, top, width, height.
pub type Region = [f64; 4];

pub struct WindowCapture {
    pub hwnd: HWND,
    d3d: ID3D11Device,
    context: ID3D11DeviceContext,
    device: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    size: SizeInt32,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
}

// The pool is free-threaded and only used from the thread that owns this value.
unsafe impl Send for WindowCapture {}

pub fn is_supported() -> bool { GraphicsCaptureSession::IsSupported().unwrap_or(false) }

fn create_device() -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext, IDirect3DDevice)> {
    unsafe {
        let (mut d3d, mut ctx) = (None, None);
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut d3d), None, Some(&mut ctx))?;
        let d3d: ID3D11Device = d3d.unwrap();
        let dxgi: IDXGIDevice = d3d.cast()?;
        let winrt: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
        Ok((d3d, ctx.unwrap(), winrt))
    }
}

impl WindowCapture {
    pub fn new(hwnd: HWND) -> windows::core::Result<WindowCapture> {
        let (d3d, context, device) = create_device()?;
        let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, size)?;
        let session = pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetIsBorderRequired(false); // no yellow border where Windows allows it
        // Windows 11 24H2+: copy League's frames into our pool at most every 900 ms instead of on every frame
        // it presents (we read one a second); older Windows ignores this and keeps the per-frame copy.
        let _ = session.SetMinUpdateInterval(windows::Foundation::TimeSpan { Duration: 9_000_000 });
        session.StartCapture()?;
        Ok(WindowCapture { hwnd, d3d, context, device, pool, session, size, staging: None })
    }

    /// Latest frame cropped to `region`. Ok(None) if no new frame yet.
    pub fn capture(&mut self, region: Region) -> windows::core::Result<Option<Frame>> {
        let Ok(frame) = self.pool.TryGetNextFrame() else { return Ok(None) };
        let content = frame.ContentSize()?;
        if content.Width != self.size.Width || content.Height != self.size.Height {
            self.size = content; // window resized: rebuild the pool
            self.pool.Recreate(&self.device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, self.size)?;
            return Ok(None);
        }
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let src: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let out = self.crop_on_gpu(&src, region);
        let _ = frame.Close();
        out
    }

    fn crop_on_gpu(&mut self, src: &ID3D11Texture2D, r: Region) -> windows::core::Result<Option<Frame>> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { src.GetDesc(&mut desc) };
        let (fw, fh) = (desc.Width.min(self.size.Width.max(0) as u32), desc.Height.min(self.size.Height.max(0) as u32));
        let x = ((fw as f64 * r[0]) as u32).min(fw);
        let y = ((fh as f64 * r[1]) as u32).min(fh);
        let w = ((fw as f64 * r[2]) as u32).min(fw - x);
        let h = ((fh as f64 * r[3]) as u32).min(fh - y);
        if w == 0 || h == 0 { return Ok(None); }

        if !matches!(self.staging, Some((_, sw, sh)) if sw == w && sh == h) {
            let sdesc = D3D11_TEXTURE2D_DESC {
                Width: w, Height: h, MipLevels: 1, ArraySize: 1, Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, Usage: D3D11_USAGE_STAGING,
                BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, MiscFlags: 0,
            };
            let mut tex = None;
            unsafe { self.d3d.CreateTexture2D(&sdesc, None, Some(&mut tex))? };
            self.staging = Some((tex.unwrap(), w, h));
        }
        let staging = self.staging.as_ref().unwrap().0.clone();
        let bx = D3D11_BOX { left: x, top: y, front: 0, right: x + w, bottom: y + h, back: 1 };
        unsafe {
            self.context.CopySubresourceRegion(&staging, 0, 0, 0, 0, src, 0, Some(&bx));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let (w, h, pitch) = (w as usize, h as usize, mapped.RowPitch as usize);
            let base = mapped.pData as *const u8;
            let mut px = vec![0u8; w * h * 4];
            for row in 0..h {
                let srow = std::slice::from_raw_parts(base.add(row * pitch), w * 4);
                px[row * w * 4..(row + 1) * w * 4].copy_from_slice(srow);
            }
            self.context.Unmap(&staging, 0);
            Ok(Some(Frame { px, w, h }))
        }
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
