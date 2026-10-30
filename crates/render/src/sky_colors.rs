//! Native jj.a(temperature), et sky color and vr fog color in display space.
fn linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
}
pub fn colors(
    temperature: f32,
    day: f32,
    rain: f32,
    thunder: f32,
    flash: f32,
) -> ([f32; 4], [f32; 4]) {
    let temp = (temperature / 3.0).clamp(-1.0, 1.0);
    let h = (0.62222224 - temp * 0.05) * 6.0;
    let saturation = 0.5 + temp * 0.1;
    let f = h - h.floor();
    let p = 1.0 - saturation;
    let q = 1.0 - saturation * f;
    let t = 1.0 - saturation * (1.0 - f);
    let rgb = match h.floor() as i32 {
        0 => [1.0, t, p],
        1 => [q, 1.0, p],
        2 => [p, 1.0, t],
        3 => [p, q, 1.0],
        4 => [t, p, 1.0],
        _ => [1.0, p, q],
    };
    // java.awt.Color first quantizes the HSB result to eight-bit RGB.
    let mut sky = rgb.map(|c| ((c * 255.0 + 0.5).floor() / 255.0) * day);
    let gray = (sky[0] * 0.3 + sky[1] * 0.59 + sky[2] * 0.11) * 0.6;
    sky = mix(sky, [gray; 3], rain * 0.75);
    let gray = (sky[0] * 0.3 + sky[1] * 0.59 + sky[2] * 0.11) * 0.2;
    sky = mix(sky, [gray; 3], thunder * 0.75);
    sky = mix(sky, [0.8, 0.8, 1.0], flash * 0.45);
    let mut fog = [
        0.7529412 * (day * 0.94 + 0.06),
        0.84705883 * (day * 0.94 + 0.06),
        day * 0.91 + 0.09,
    ];
    fog[0] *= 1.0 - rain * 0.5;
    fog[1] *= 1.0 - rain * 0.5;
    fog[2] *= 1.0 - rain * 0.4;
    fog = fog.map(|v| v * (1.0 - thunder * 0.5));
    (
        [linear(sky[0]), linear(sky[1]), linear(sky[2]), 1.0],
        [linear(fog[0]), linear(fog[1]), linear(fog[2]), 1.0],
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn night_is_dark_and_weather_desaturates_native_sky() {
        let (day, _) = colors(0.5, 1.0, 0.0, 0.0, 0.0);
        let (night, fog) = colors(0.5, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(night, [0.0, 0.0, 0.0, 1.0]);
        assert!(fog[2] < 0.01);
        let (rain, _) = colors(0.5, 1.0, 1.0, 0.0, 0.0);
        assert!(rain[2] < day[2]);
        let (flash, _) = colors(0.5, 0.0, 0.0, 0.0, 1.0);
        assert!(flash[2] > 0.1);
    }
}
