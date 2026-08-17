use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use wasmtime::{
    Engine, Store,
    component::{Component, HasSelf, Linker},
};

use crate::{
    bindings::{gateway, inventory, order, product},
    db::{DbPool, HostState},
};

pub struct ComponentRuntime {
    engine: Engine,
    gateway: Component,
    product: Component,
    inventory: Component,
    order: Component,
    pool: DbPool,
}

impl ComponentRuntime {
    pub fn load(pool: DbPool) -> Result<Self> {
        let engine = Engine::default();
        let component_dir = component_dir();

        Ok(Self {
            gateway: load_component(&engine, &component_dir, "gateway_component.wasm")?,
            product: load_component(&engine, &component_dir, "product_agent_component.wasm")?,
            inventory: load_component(&engine, &component_dir, "inventory_agent_component.wasm")?,
            order: load_component(&engine, &component_dir, "order_agent_component.wasm")?,
            engine,
            pool,
        })
    }

    pub fn resolve_route(
        &self,
        method: &str,
        path: &str,
    ) -> Result<gateway::exports::ehlung::commerce::gateway::RouteDecision> {
        let linker = Linker::new(&self.engine);
        let mut store = self.store();
        let bindings = gateway::GatewayComponent::instantiate(&mut store, &self.gateway, &linker)
            .map_err(|error| anyhow!("Gateway Component 인스턴스 생성 실패: {error}"))?;
        bindings
            .ehlung_commerce_gateway()
            .call_resolve(&mut store, method, path)
            .map_err(|error| anyhow!("Gateway Component 호출 실패: {error}"))
    }

    pub fn list_products(
        &self,
    ) -> Result<Result<Vec<product::ehlung::commerce::models::Product>, String>> {
        let mut linker = Linker::new(&self.engine);
        product::ProductComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| anyhow!("상품 저장소 기능 등록 실패: {error}"))?;
        let mut store = self.store();
        let bindings = product::ProductComponent::instantiate(&mut store, &self.product, &linker)
            .map_err(|error| anyhow!("Product Component 인스턴스 생성 실패: {error}"))?;
        bindings
            .ehlung_commerce_product_agent()
            .call_list(&mut store)
            .map_err(|error| anyhow!("Product Component 호출 실패: {error}"))
    }

    pub fn create_product(
        &self,
        draft: product::ehlung::commerce::models::ProductDraft,
    ) -> Result<Result<product::ehlung::commerce::models::Product, String>> {
        let mut linker = Linker::new(&self.engine);
        product::ProductComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| anyhow!("상품 저장소 기능 등록 실패: {error}"))?;
        let mut store = self.store();
        let bindings = product::ProductComponent::instantiate(&mut store, &self.product, &linker)
            .map_err(|error| anyhow!("Product Component 인스턴스 생성 실패: {error}"))?;
        bindings
            .ehlung_commerce_product_agent()
            .call_create(&mut store, &draft)
            .map_err(|error| anyhow!("Product Component 호출 실패: {error}"))
    }

    pub fn adjust_stock(
        &self,
        product_id: u64,
        delta: i64,
    ) -> Result<Result<inventory::ehlung::commerce::models::Product, String>> {
        let mut linker = Linker::new(&self.engine);
        inventory::InventoryComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| anyhow!("재고 저장소 기능 등록 실패: {error}"))?;
        let mut store = self.store();
        let bindings =
            inventory::InventoryComponent::instantiate(&mut store, &self.inventory, &linker)
                .map_err(|error| anyhow!("Inventory Component 인스턴스 생성 실패: {error}"))?;
        bindings
            .ehlung_commerce_inventory_agent()
            .call_adjust_stock(&mut store, product_id, delta)
            .map_err(|error| anyhow!("Inventory Component 호출 실패: {error}"))
    }

    pub fn create_order(
        &self,
        input: order::ehlung::commerce::models::OrderInput,
    ) -> Result<Result<order::ehlung::commerce::models::Order, String>> {
        let mut linker = Linker::new(&self.engine);
        order::OrderComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| anyhow!("주문 저장소 기능 등록 실패: {error}"))?;
        let mut store = self.store();
        let bindings = order::OrderComponent::instantiate(&mut store, &self.order, &linker)
            .map_err(|error| anyhow!("Order Component 인스턴스 생성 실패: {error}"))?;
        bindings
            .ehlung_commerce_order_agent()
            .call_create(&mut store, &input)
            .map_err(|error| anyhow!("Order Component 호출 실패: {error}"))
    }

    pub fn get_order(
        &self,
        order_id: u64,
    ) -> Result<Result<Option<order::ehlung::commerce::models::Order>, String>> {
        let mut linker = Linker::new(&self.engine);
        order::OrderComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| anyhow!("주문 저장소 기능 등록 실패: {error}"))?;
        let mut store = self.store();
        let bindings = order::OrderComponent::instantiate(&mut store, &self.order, &linker)
            .map_err(|error| anyhow!("Order Component 인스턴스 생성 실패: {error}"))?;
        bindings
            .ehlung_commerce_order_agent()
            .call_get(&mut store, order_id)
            .map_err(|error| anyhow!("Order Component 호출 실패: {error}"))
    }

    pub fn cancel_order(
        &self,
        order_id: u64,
    ) -> Result<Result<order::ehlung::commerce::models::Order, String>> {
        let mut linker = Linker::new(&self.engine);
        order::OrderComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| anyhow!("주문 저장소 기능 등록 실패: {error}"))?;
        let mut store = self.store();
        let bindings = order::OrderComponent::instantiate(&mut store, &self.order, &linker)
            .map_err(|error| anyhow!("Order Component 인스턴스 생성 실패: {error}"))?;
        bindings
            .ehlung_commerce_order_agent()
            .call_cancel(&mut store, order_id)
            .map_err(|error| anyhow!("Order Component 호출 실패: {error}"))
    }

    fn store(&self) -> Store<HostState> {
        Store::new(
            &self.engine,
            HostState {
                pool: self.pool.clone(),
            },
        )
    }
}

fn component_dir() -> PathBuf {
    env::var_os("COMPONENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/wasm32-wasip2/release")
        })
}

fn load_component(engine: &Engine, dir: &Path, file: &str) -> Result<Component> {
    let path = dir.join(file);
    Component::from_file(engine, &path).map_err(|error| {
        anyhow!(
            "Wasm Component를 불러오지 못했습니다 ({}): {error}",
            path.display()
        )
    })
}
