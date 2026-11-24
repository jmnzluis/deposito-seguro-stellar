# Depósito Seguro (`deposito-seguro-stellar`)

Contrato inteligente de **custodia de depósitos de garantía y señas (escrow)** construido en Rust con `soroban-sdk` v30 para la red **Stellar**, orientado a alquileres, cabinas y tours en Costa Rica.

## 1. El problema

En Costa Rica es muy común pagar señas por adelantado para reservar cabinas, tours o entregar depósitos de garantía en alquileres mediante transferencias directas o SINPE Móvil. Sin embargo, este esquema genera desconfianza para ambas partes:

- **Para el cliente:** una vez enviado el dinero, no existe garantía técnica de que el proveedor cumpla con el servicio o devuelva el depósito de garantía al finalizar el alquiler.
- **Para el proveedor:** si no solicita un adelanto o garantía, corre el riesgo de cancelaciones de último minuto o daños sin respaldo.
- **En caso de desacuerdo:** resolver un reclamo de forma informal suele ser lento, costoso y depende únicamente de la buena voluntad de quien ya tiene los fondos.

## 2. Por qué elegí este problema

Una transferencia bancaria tradicional es **irreversible y unilateral**: en cuanto el dinero sale de la cuenta del cliente, el control pasa por completo al receptor.

Un contrato inteligente tipo **escrow** en Stellar aporta valor real frente a una transferencia tradicional porque:

- **Custodia neutral:** los fondos quedan bloqueados en el contrato inteligente, no en la billetera del cliente ni en la del proveedor.
- **Reglas transparentes e inmutables:** las condiciones de liberación, reembolso y fecha límite se ejecutan automáticamente en la red.
- **Protección por vencimiento de plazo:** si pasa la `fecha_limite` sin que se haya liberado el pago ni abierto una disputa, el cliente puede recuperar sus fondos directamente.
- **Arbitraje acotado:** un tercero designado (`arbitro`) solo puede intervenir si alguna de las partes abre formalmente una disputa, sin poder mover fondos en acuerdos normales.

## 3. Cómo funciona el contrato

### Actores

| Actor | Rol en el acuerdo |
| --- | --- |
| **Cliente** (`cliente`) | Crea y fondea el depósito, confirma y libera los fondos al proveedor, puede reclamar reembolso tras vencer `fecha_limite` y puede abrir disputa. |
| **Proveedor** (`proveedor`) | Recibe los fondos al liberarse el depósito, puede devolver el dinero al cliente en cualquier momento antes del vencimiento y puede abrir disputa. |
| **Árbitro** (`arbitro`) | Tercero neutral designado al crear el acuerdo. Solo puede resolver depósitos en estado `EnDisputa`, decidiendo si los fondos van al proveedor o regresan al cliente. |

### Estados del depósito (`EstadoDeposito`)

- **`Activo`**: el depósito fue creado, los fondos están custodiados en el contrato y el acuerdo está vigente.
- **`Liberado`**: los fondos fueron transferidos al proveedor (por confirmación del cliente o fallo del árbitro).
- **`Reembolsado`**: los fondos fueron devueltos al cliente (por devolución del proveedor, vencimiento de `fecha_limite` o fallo del árbitro).
- **`EnDisputa`**: el cliente o el proveedor bloqueó el flujo normal para que únicamente el árbitro resuelva el destino de los fondos.

### Funciones exportadas

| Función | Parámetros | Retorno | Descripción |
| --- | --- | --- | --- |
| `crear_deposito` | `cliente`, `proveedor`, `arbitro`, `token`, `monto`, `fecha_limite` | `Result<u64, Error>` | Autentica al cliente, valida monto, fecha límite y participantes distintos, transfiere el token al contrato, asigna un ID incremental y emite `DepositoCreado`. |
| `liberar_fondos` | `id` | `Result<(), Error>` | Requiere autorización del cliente sobre un depósito `Activo`, cambia el estado a `Liberado`, transfiere los fondos al proveedor y emite `FondosLiberados`. |
| `reembolsar` | `id` | `Result<(), Error>` | Devuelve los fondos al cliente desde un depósito `Activo`. Antes o en `fecha_limite` requiere autorización del proveedor; después de `fecha_limite` (`timestamp > fecha_limite`) requiere autorización del cliente. Emite `FondosReembolsados`. |
| `abrir_disputa` | `id`, `solicitante` | `Result<(), Error>` | Permite únicamente al cliente o al proveedor (autenticado) pasar un depósito `Activo` a `EnDisputa` y emite `DisputaAbierta`. |
| `resolver_disputa` | `id`, `entregar_a_proveedor` | `Result<(), Error>` | Requiere autorización del árbitro sobre un depósito `EnDisputa`. Transfiere los fondos al proveedor (`true` → `Liberado`) o al cliente (`false` → `Reembolsado`) y emite `DisputaResuelta`. |
| `obtener_deposito` | `id` | `Result<Deposito, Error>` | Consulta y devuelve la estructura completa del depósito almacenado para el `id` indicado. |

## 4. Despliegue en Stellar Testnet

- **Contract ID:** `CCQI77HF7LI3OSSNGLRQKJ4U6CW5BHU4ROTPIWSAF2KJYUANTJAYHTEF`
- **Wasm Hash:** `2dfb181da6e344ce7489aed66ccb76c6e27c9e375b9fc142352ea33e8cbbb2ae`
- **Stellar Lab:** [Ver contrato en Stellar Lab](https://lab.stellar.org/r/testnet/contract/CCQI77HF7LI3OSSNGLRQKJ4U6CW5BHU4ROTPIWSAF2KJYUANTJAYHTEF)
- **Stellar Expert:** [Ver transacción de despliegue en Stellar Expert](https://stellar.expert/explorer/testnet/tx/9ab969f03184c65577a711bf2ac120a749f63e713c9ea13bf7f5467949c2000c)

## 5. Pruebas implementadas

Las pruebas unitarias están en `src/test.rs` y utilizan un token Stellar Asset Contract (SAC) de prueba junto con control de autorizaciones (`MockAuth`) y reloj del ledger (`env.ledger().set_timestamp`):

1. **`test_flujo_feliz_crear_y_liberar`**: verifica la creación de un depósito, la transferencia de fondos al contrato, la consulta con `obtener_deposito`, la liberación al proveedor por parte del cliente, el cambio de estado a `Liberado` y la asignación de IDs incrementales (`1`, `2`).
2. **`test_reembolso_por_vencimiento_de_plazo`**: comprueba que el cliente no puede reembolsar antes de `fecha_limite`, y que al avanzar el timestamp del ledger por encima de `fecha_limite` sí puede recuperar sus fondos y el estado pasa a `Reembolsado`.
3. **`test_reembolso_por_proveedor_antes_de_vencimiento`**: valida que el proveedor puede devolver voluntariamente los fondos al cliente antes de que venza el plazo.
4. **`test_rechazo_acciones_sin_autorizacion`**: verifica que el proveedor no puede llamar a `liberar_fondos`, que un tercero o el árbitro no pueden abrir disputa (`Error::NoAutorizado`), y que ni el cliente ni un tercero pueden ejecutar `resolver_disputa`.
5. **`test_resolucion_disputa_por_arbitro`**: comprueba ambos escenarios de resolución por el árbitro: entrega al proveedor (`entregar_a_proveedor = true` → `Liberado`) y devolución al cliente (`entregar_a_proveedor = false` → `Reembolsado`).

Resultado esperado al ejecutar `cargo test`:

```text
running 5 tests
test test::test_reembolso_por_proveedor_antes_de_vencimiento ... ok
test test::test_rechazo_acciones_sin_autorizacion ... ok
test test::test_reembolso_por_vencimiento_de_plazo ... ok
test test::test_flujo_feliz_crear_y_liberar ... ok
test test::test_resolucion_disputa_por_arbitro ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## 6. Cómo compilar y probar

### Requisitos

- **Rust** (con `cargo` y `rustup`)
- Target WebAssembly para Soroban:
  ```bash
  rustup target add wasm32v1-none
  ```
- **Stellar CLI** (`stellar-cli` v30.0.1 o compatible)

### Comandos

Ejecutar las pruebas unitarias:

```bash
cargo test
```

Compilar y optimizar el contrato a WASM:

```bash
stellar contract build
```

El binario optimizado se genera en `target/wasm32v1-none/release/deposito_seguro.wasm`.

---

**Autor:** Luis Alonso Jiménez Ramírez
