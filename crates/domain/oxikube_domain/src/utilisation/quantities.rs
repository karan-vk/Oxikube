//! Reading quantities out of `resources` maps.

use super::{GpuVendors, ResourceAmounts};
use crate::json::JsonRef;
use crate::quantity::Quantity;

/// The resource name of ephemeral storage.
pub(super) const EPHEMERAL: &str = "ephemeral-storage";

/// `map[key]` as a quantity: a string (`"250m"`) or a bare JSON number. `None` when absent,
/// unparseable or negative (the API server rejects negative quantities, so a negative one is
/// garbage, not a credit).
pub(super) fn quantity_of(map: JsonRef<'_>, key: &str) -> Option<Quantity> {
    let value = map.get(key)?;
    let quantity = if let Some(text) = value.as_str() {
        Quantity::parse(text).ok()?
    } else if let Some(n) = value.as_i64() {
        Quantity::from_value(n)
    } else {
        Quantity::parse(&value.as_f64()?.to_string()).ok()?
    };
    (!quantity.is_negative()).then_some(quantity)
}

/// A quantity as a whole count: rounded up like `Quantity::value`, saturating.
pub(super) fn whole_count(quantity: &Quantity) -> u64 {
    u64::try_from(quantity.value()).unwrap_or(u64::MAX)
}

/// The CPU, memory, ephemeral storage and configured GPU entries of one `requests` / `limits` /
/// `allocatable` map. Anything else in the map (`hugepages-*`, `pods`) is ignored.
pub(super) fn amounts_of(map: JsonRef<'_>, gpus: &GpuVendors) -> ResourceAmounts {
    let mut out = ResourceAmounts::zero();
    if !map.is_object() {
        return out;
    }
    out.cpu = quantity_of(map, "cpu").unwrap_or_default();
    out.memory = quantity_of(map, "memory").unwrap_or_default();
    out.ephemeral = quantity_of(map, EPHEMERAL).unwrap_or_default();
    for name in gpus.names() {
        if let Some(count) = quantity_of(map, name).map(|q| whole_count(&q))
            && count > 0
        {
            out.gpu.insert(name.clone(), count);
        }
    }
    out
}

/// Replaces the amounts of `dst` by those `map` sets (an unset resource keeps its value).
pub(super) fn override_with(dst: &mut ResourceAmounts, map: JsonRef<'_>, gpus: &GpuVendors) {
    if !map.is_object() {
        return;
    }
    if let Some(cpu) = quantity_of(map, "cpu") {
        dst.cpu = cpu;
    }
    if let Some(memory) = quantity_of(map, "memory") {
        dst.memory = memory;
    }
    if let Some(ephemeral) = quantity_of(map, EPHEMERAL) {
        dst.ephemeral = ephemeral;
    }
    for name in gpus.names() {
        if let Some(count) = quantity_of(map, name).map(|q| whole_count(&q)) {
            if count == 0 {
                dst.gpu.remove(name);
            } else {
                dst.gpu.insert(name.clone(), count);
            }
        }
    }
}
