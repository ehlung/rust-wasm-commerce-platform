wit_bindgen::generate!({
    path: "../../wit",
    world: "inventory-component",
});

use ehlung::commerce::{inventory_repository, models::Product};
use exports::ehlung::commerce::inventory_agent::Guest;

struct InventoryAgent;

impl Guest for InventoryAgent {
    fn adjust_stock(product_id: u64, delta: i64) -> Result<Product, String> {
        validate_adjustment(product_id, delta)?;
        inventory_repository::adjust_stock(product_id, delta)
    }
}

fn validate_adjustment(product_id: u64, delta: i64) -> Result<(), String> {
    if product_id == 0 {
        return Err("상품 ID는 0보다 커야 합니다.".to_string());
    }
    if delta == 0 {
        return Err("재고 변경량은 0일 수 없습니다.".to_string());
    }
    if !(-10_000..=10_000).contains(&delta) {
        return Err("한 번에 변경할 수 있는 재고는 -10,000~10,000입니다.".to_string());
    }
    Ok(())
}

export!(InventoryAgent);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_adjustment() {
        assert!(validate_adjustment(1, -3).is_ok());
    }

    #[test]
    fn rejects_noop_adjustment() {
        assert!(validate_adjustment(1, 0).is_err());
    }
}
