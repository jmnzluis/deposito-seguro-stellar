#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    Address, Env, IntoVal,
};

struct TestSetup<'a> {
    env: Env,
    contract_id: Address,
    client: DepositoSeguroContractClient<'a>,
    token_id: Address,
    token_client: TokenClient<'a>,
    cliente: Address,
    proveedor: Address,
    arbitro: Address,
}

fn setup<'a>() -> TestSetup<'a> {
    let env = Env::default();
    env.ledger().set_timestamp(1_000);

    let contract_id = env.register(DepositoSeguroContract, ());
    let client = DepositoSeguroContractClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(token_admin);
    let token_id = sac.address();
    let token_client = TokenClient::new(&env, &token_id);
    let sac_client = StellarAssetClient::new(&env, &token_id);

    let cliente = Address::generate(&env);
    let proveedor = Address::generate(&env);
    let arbitro = Address::generate(&env);

    env.mock_all_auths();
    sac_client.mint(&cliente, &10_000);

    TestSetup {
        env,
        contract_id,
        client,
        token_id,
        token_client,
        cliente,
        proveedor,
        arbitro,
    }
}

#[test]
fn test_flujo_feliz_crear_y_liberar() {
    let s = setup();
    let monto = 2_500i128;
    let fecha_limite = 5_000u64;

    let id = s.client.crear_deposito(
        &s.cliente,
        &s.proveedor,
        &s.arbitro,
        &s.token_id,
        &monto,
        &fecha_limite,
    );
    assert_eq!(id, 1);

    // Verifica que el saldo se transfirió al contrato y el acuerdo se guardó correctamente
    assert_eq!(s.token_client.balance(&s.cliente), 7_500);
    assert_eq!(s.token_client.balance(&s.contract_id), 2_500);

    let deposito = s.client.obtener_deposito(&id);
    assert_eq!(deposito.id, 1);
    assert_eq!(deposito.cliente, s.cliente);
    assert_eq!(deposito.proveedor, s.proveedor);
    assert_eq!(deposito.arbitro, s.arbitro);
    assert_eq!(deposito.monto, monto);
    assert_eq!(deposito.fecha_limite, fecha_limite);
    assert_eq!(deposito.estado, EstadoDeposito::Activo);

    // El cliente libera los fondos al proveedor
    s.client.liberar_fondos(&id);

    assert_eq!(s.token_client.balance(&s.contract_id), 0);
    assert_eq!(s.token_client.balance(&s.proveedor), 2_500);

    let deposito_final = s.client.obtener_deposito(&id);
    assert_eq!(deposito_final.estado, EstadoDeposito::Liberado);

    // Un segundo depósito recibe ID incremental = 2
    let id_2 = s.client.crear_deposito(
        &s.cliente,
        &s.proveedor,
        &s.arbitro,
        &s.token_id,
        &1_000,
        &fecha_limite,
    );
    assert_eq!(id_2, 2);
}

#[test]
fn test_reembolso_por_vencimiento_de_plazo() {
    let s = setup();
    let monto = 3_000i128;
    let fecha_limite = 2_000u64;

    let id = s.client.crear_deposito(
        &s.cliente,
        &s.proveedor,
        &s.arbitro,
        &s.token_id,
        &monto,
        &fecha_limite,
    );

    // Antes del vencimiento, el cliente NO puede reembolsar por sí mismo (se requiere auth del proveedor)
    s.env.mock_auths(&[MockAuth {
        address: &s.cliente,
        invoke: &MockAuthInvoke {
            contract: &s.contract_id,
            fn_name: "reembolsar",
            args: (id,).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    assert!(s.client.try_reembolsar(&id).is_err());

    // Avanzamos el timestamp del ledger más allá de fecha_limite
    s.env.ledger().set_timestamp(2_001);

    // Ahora el cliente sí puede reclamar el reembolso con su propia autorización
    s.env.mock_auths(&[MockAuth {
        address: &s.cliente,
        invoke: &MockAuthInvoke {
            contract: &s.contract_id,
            fn_name: "reembolsar",
            args: (id,).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    s.client.reembolsar(&id);

    assert_eq!(s.token_client.balance(&s.contract_id), 0);
    assert_eq!(s.token_client.balance(&s.cliente), 10_000);
    assert_eq!(
        s.client.obtener_deposito(&id).estado,
        EstadoDeposito::Reembolsado
    );
}

#[test]
fn test_reembolso_por_proveedor_antes_de_vencimiento() {
    let s = setup();
    let id = s.client.crear_deposito(
        &s.cliente,
        &s.proveedor,
        &s.arbitro,
        &s.token_id,
        &1_500,
        &5_000,
    );

    // El proveedor puede devolver los fondos en cualquier momento antes de fecha_limite
    s.env.mock_auths(&[MockAuth {
        address: &s.proveedor,
        invoke: &MockAuthInvoke {
            contract: &s.contract_id,
            fn_name: "reembolsar",
            args: (id,).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    s.client.reembolsar(&id);

    assert_eq!(s.token_client.balance(&s.cliente), 10_000);
    assert_eq!(
        s.client.obtener_deposito(&id).estado,
        EstadoDeposito::Reembolsado
    );
}

#[test]
fn test_rechazo_acciones_sin_autorizacion() {
    let s = setup();
    let intruso = Address::generate(&s.env);
    let id = s.client.crear_deposito(
        &s.cliente,
        &s.proveedor,
        &s.arbitro,
        &s.token_id,
        &2_000,
        &5_000,
    );

    // 1. El proveedor no puede liberar los fondos (solo el cliente)
    s.env.mock_auths(&[MockAuth {
        address: &s.proveedor,
        invoke: &MockAuthInvoke {
            contract: &s.contract_id,
            fn_name: "liberar_fondos",
            args: (id,).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    assert!(s.client.try_liberar_fondos(&id).is_err());

    // 2. Un tercero (ni cliente ni proveedor) no puede abrir disputa -> Error::NoAutorizado
    s.env.mock_all_auths();
    assert_eq!(
        s.client.try_abrir_disputa(&id, &intruso),
        Err(Ok(Error::NoAutorizado))
    );
    assert_eq!(
        s.client.try_abrir_disputa(&id, &s.arbitro),
        Err(Ok(Error::NoAutorizado))
    );

    // 3. El cliente abre la disputa correctamente
    s.client.abrir_disputa(&id, &s.cliente);
    assert_eq!(
        s.client.obtener_deposito(&id).estado,
        EstadoDeposito::EnDisputa
    );

    // 4. Ni el cliente ni un intruso pueden resolver la disputa (solo el árbitro)
    s.env.mock_auths(&[MockAuth {
        address: &s.cliente,
        invoke: &MockAuthInvoke {
            contract: &s.contract_id,
            fn_name: "resolver_disputa",
            args: (id, false).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    assert!(s.client.try_resolver_disputa(&id, &false).is_err());

    s.env.mock_auths(&[MockAuth {
        address: &intruso,
        invoke: &MockAuthInvoke {
            contract: &s.contract_id,
            fn_name: "resolver_disputa",
            args: (id, true).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    assert!(s.client.try_resolver_disputa(&id, &true).is_err());

    // El depósito sigue en disputa y los fondos intactos en el contrato
    assert_eq!(
        s.client.obtener_deposito(&id).estado,
        EstadoDeposito::EnDisputa
    );
    assert_eq!(s.token_client.balance(&s.contract_id), 2_000);
}

#[test]
fn test_resolucion_disputa_por_arbitro() {
    let s = setup();

    // Caso 1: el árbitro resuelve a favor del proveedor
    let id_1 = s.client.crear_deposito(
        &s.cliente,
        &s.proveedor,
        &s.arbitro,
        &s.token_id,
        &2_000,
        &5_000,
    );
    s.client.abrir_disputa(&id_1, &s.proveedor);

    s.env.mock_auths(&[MockAuth {
        address: &s.arbitro,
        invoke: &MockAuthInvoke {
            contract: &s.contract_id,
            fn_name: "resolver_disputa",
            args: (id_1, true).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    s.client.resolver_disputa(&id_1, &true);

    assert_eq!(s.token_client.balance(&s.proveedor), 2_000);
    assert_eq!(
        s.client.obtener_deposito(&id_1).estado,
        EstadoDeposito::Liberado
    );

    // Caso 2: el árbitro resuelve a favor del cliente (reembolso)
    s.env.mock_all_auths();
    let id_2 = s.client.crear_deposito(
        &s.cliente,
        &s.proveedor,
        &s.arbitro,
        &s.token_id,
        &3_000,
        &5_000,
    );
    s.client.abrir_disputa(&id_2, &s.cliente);

    s.env.mock_auths(&[MockAuth {
        address: &s.arbitro,
        invoke: &MockAuthInvoke {
            contract: &s.contract_id,
            fn_name: "resolver_disputa",
            args: (id_2, false).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    s.client.resolver_disputa(&id_2, &false);

    // Cliente inició con 10_000, pagó 2_000 (id_1) y recuperó 3_000 (id_2) -> 8_000
    assert_eq!(s.token_client.balance(&s.cliente), 8_000);
    assert_eq!(s.token_client.balance(&s.contract_id), 0);
    assert_eq!(
        s.client.obtener_deposito(&id_2).estado,
        EstadoDeposito::Reembolsado
    );
}
