use windows::Win32::{
    Foundation::RPC_E_CHANGED_MODE,
    Media::Audio::{
        eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
        MMDeviceEnumerator,
    },
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    },
};

struct ComApartment {
    uninitialize: bool,
}

impl ComApartment {
    fn enter() -> Result<Self, String> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result == RPC_E_CHANGED_MODE {
            // The current thread already belongs to another COM apartment.
            return Ok(Self {
                uninitialize: false,
            });
        }
        result
            .ok()
            .map_err(|error| format!("Não foi possível iniciar o áudio do Windows. {error}"))?;
        Ok(Self { uninitialize: true })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

fn with_endpoint(
    operation: impl FnOnce(&IAudioEndpointVolume) -> Result<u8, String>,
) -> Result<u8, String> {
    let _apartment = ComApartment::enter()?;
    let devices: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER) }.map_err(
            |error| format!("Não foi possível consultar os dispositivos de áudio. {error}"),
        )?;
    let playback = unsafe { devices.GetDefaultAudioEndpoint(eRender, eMultimedia) }
        .map_err(|error| format!("Saída de áudio padrão indisponível. {error}"))?;
    let volume = unsafe { playback.Activate(CLSCTX_INPROC_SERVER, None) }
        .map_err(|error| format!("Não foi possível acessar o volume da saída padrão. {error}"))?;
    operation(&volume)
}

fn read_level(endpoint: &IAudioEndpointVolume) -> Result<f32, String> {
    unsafe { endpoint.GetMasterVolumeLevelScalar() }
        .map_err(|error| format!("Não foi possível ler o volume. {error}"))
}

fn display_percent(level: f32) -> Result<u8, String> {
    if !level.is_finite() {
        return Err("O Windows informou um nível de volume inválido.".into());
    }
    Ok((level.clamp(0.0, 1.0) * 100.0).round() as u8)
}

fn relative_level(level: f32, step: i8) -> Result<f32, String> {
    if !level.is_finite() {
        return Err("O Windows informou um nível de volume inválido.".into());
    }
    Ok((level + f32::from(step) / 100.0).clamp(0.0, 1.0))
}

pub fn current() -> Result<u8, String> {
    with_endpoint(|endpoint| display_percent(read_level(endpoint)?))
}

/// Adjust the volume of the current default playback device by percentage points.
pub fn adjust(step: i8) -> Result<u8, String> {
    if step == 0 {
        return current();
    }
    with_endpoint(|endpoint| {
        let old = read_level(endpoint)?;
        let new = relative_level(old, step)?;
        if new != old {
            unsafe { endpoint.SetMasterVolumeLevelScalar(new, std::ptr::null()) }
                .map_err(|error| format!("Não foi possível ajustar o volume. {error}"))?;
        }
        display_percent(read_level(endpoint)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precise_adjustments_are_percentage_points_and_stay_in_range() {
        let increased = relative_level(0.373, 5).unwrap();
        assert!((increased - 0.423).abs() < 0.00001);
        assert_eq!(display_percent(increased).unwrap(), 42);
        assert_eq!(relative_level(0.98, 5), Ok(1.0));
        assert_eq!(relative_level(0.02, -5), Ok(0.0));
        assert!(relative_level(f32::NAN, 5).is_err());
    }
}
