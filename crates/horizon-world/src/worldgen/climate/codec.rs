use super::super::{DecodeError, Object, RawValue, number_text, required};
use super::{Parameter, ParameterPoint, quantize};

fn float(raw: &RawValue, min: f32, max: f32) -> Result<f32, DecodeError> {
    let value: f32 = number_text(raw)?
        .parse()
        .map_err(|_| DecodeError::Field("climate float"))?;
    if value.total_cmp(&min).is_lt() || value.total_cmp(&max).is_gt() {
        return Err(DecodeError::Field("climate float bounds"));
    }
    Ok(value)
}
fn parameter(raw: &RawValue) -> Result<Parameter, DecodeError> {
    let (min, max) = if raw.get().starts_with('[') {
        let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
        if values.len() != 2 {
            return Err(DecodeError::Field("climate interval length"));
        }
        (float(&values[0], -2., 2.)?, float(&values[1], -2., 2.)?)
    } else if raw.get().starts_with('{') {
        let object: Object = serde_json::from_str(raw.get())?;
        (
            float(required(&object, "min")?, -2., 2.)?,
            float(required(&object, "max")?, -2., 2.)?,
        )
    } else {
        let value = float(raw, -2., 2.)?;
        (value, value)
    };
    if min.total_cmp(&max).is_gt() {
        return Err(DecodeError::Field("climate interval order"));
    }
    Ok(Parameter::new(quantize(min), quantize(max)))
}
/// Vanilla's scalar, two-element list or min/max object climate interval codec.
pub fn decode_parameter(json: &str) -> Result<Parameter, DecodeError> {
    let raw: Box<RawValue> = serde_json::from_str(json)?;
    parameter(&raw)
}
/// Six required climate intervals and a required offset in [0,1].
pub fn decode_parameter_point(json: &str) -> Result<ParameterPoint, DecodeError> {
    let object: Object = serde_json::from_str(json)?;
    let mut coordinates = [Parameter::new(0, 0); 6];
    for (coordinate, name) in coordinates.iter_mut().zip([
        "temperature",
        "humidity",
        "continentalness",
        "erosion",
        "depth",
        "weirdness",
    ]) {
        *coordinate = parameter(required(&object, name)?)?;
    }
    let offset = quantize(float(required(&object, "offset")?, 0., 1.)?);
    Ok(ParameterPoint {
        coordinates,
        offset,
    })
}
