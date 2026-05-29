use jaq_core::load;
use jaq_core::native::Fun;
use jaq_core::DataT;

use crate::core::QueryValue;

/// JSONB-specific jaq definitions.
pub fn defs() -> impl Iterator<Item = load::parse::Def<&'static str>> {
    std::iter::empty()
}

/// JSONB-specific jaq native functions.
pub fn funs<D>() -> impl Iterator<Item = Fun<D>>
where
    D: for<'a> DataT<V<'a> = QueryValue<'a>>,
{
    std::iter::empty()
}
