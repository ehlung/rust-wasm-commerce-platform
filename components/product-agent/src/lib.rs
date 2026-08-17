wit_bindgen::generate!({
    path: "../../wit",
    world: "product-component",
});

use ehlung::commerce::{
    models::{Product, ProductDraft},
    product_repository,
};
use exports::ehlung::commerce::product_agent::Guest;

struct ProductAgent;

impl Guest for ProductAgent {
    fn list() -> Result<Vec<Product>, String> {
        product_repository::list()
    }

    fn create(mut draft: ProductDraft) -> Result<Product, String> {
        validate_and_normalize(&mut draft)?;
        product_repository::create(&draft)
    }
}

fn validate_and_normalize(draft: &mut ProductDraft) -> Result<(), String> {
    draft.sku = draft.sku.trim().to_ascii_uppercase();
    draft.name = draft.name.trim().to_string();

    if draft.sku.len() < 3 || draft.sku.len() > 40 {
        return Err("SKU는 3~40자로 입력해야 합니다.".to_string());
    }
    if !draft
        .sku
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("SKU에는 영문, 숫자, 하이픈, 밑줄만 사용할 수 있습니다.".to_string());
    }
    if draft.name.len() < 2 || draft.name.len() > 120 {
        return Err("상품명은 2~120자로 입력해야 합니다.".to_string());
    }
    if draft.price_cents == 0 {
        return Err("상품 가격은 0보다 커야 합니다.".to_string());
    }
    Ok(())
}

export!(ProductAgent);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_product_fields() {
        let mut draft = ProductDraft {
            sku: "  wasm-book_01 ".into(),
            name: "  WebAssembly 실전  ".into(),
            price_cents: 25_000,
            initial_stock: 10,
        };
        validate_and_normalize(&mut draft).unwrap();
        assert_eq!(draft.sku, "WASM-BOOK_01");
        assert_eq!(draft.name, "WebAssembly 실전");
    }

    #[test]
    fn rejects_zero_price() {
        let mut draft = ProductDraft {
            sku: "TEST-01".into(),
            name: "테스트 상품".into(),
            price_cents: 0,
            initial_stock: 0,
        };
        assert!(validate_and_normalize(&mut draft).is_err());
    }
}
