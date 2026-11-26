use std::process::Command;

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

fn wpctl(args: &[&str]) -> Result<String, String> {
    let output = Command::new("wpctl")
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| format!("PipeWire/WirePlumber (wpctl): {error}"))?;
    if !output.status.success() {
        return Err(format!("PipeWire/WirePlumber (wpctl): {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn level(output: &str) -> Option<f32> {
    output.strip_prefix("Volume:")?.split_whitespace().next()?.parse::<f32>()
        .ok().filter(|value| value.is_finite())
}

fn read() -> Result<f32, String> {
    level(&wpctl(&["get-volume", SINK])?).ok_or("Não foi possível ler o volume.".into())
}

pub fn current() -> Result<u8, String> {
    Ok((read()?.clamp(0., 1.) * 100.).round() as u8)
}

pub fn adjust(step: i8) -> Result<u8, String> {
    let value = (read()? + f32::from(step) / 100.).clamp(0., 1.);
    wpctl(&["set-volume", SINK, &format!("{value:.4}")])?;
    current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pipewire_scalars_and_retains_mute_flag_outside_volume() {
        assert_eq!(level("Volume: 0.47 [MUTED]\n"), Some(0.47));
        assert_eq!(level("Volume: NaN"), None);
        assert_eq!(level("Failed"), None);
    }
}
