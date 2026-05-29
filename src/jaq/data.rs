use jaq_core::data::DataT;

use crate::core::QueryValue;

/// jaq data marker for JSONB-backed values.
pub struct JsonbData;

impl DataT for JsonbData {
    type V<'a> = QueryValue<'a>;
    type Data<'a> = &'a jaq_core::Lut<Self>;
}
