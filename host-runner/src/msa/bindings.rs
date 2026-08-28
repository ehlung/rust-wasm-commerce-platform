pub mod gateway {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "gateway-component",
    });
}

pub mod product {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "product-component",
    });
}

pub mod inventory {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "inventory-component",
    });
}

pub mod order {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "order-component",
    });
}
