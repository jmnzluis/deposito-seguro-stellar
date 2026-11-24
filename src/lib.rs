#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token::TokenClient,
    Address, Env,
};

const DAY_IN_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;
const TTL_EXTEND_TO: u32 = 60 * DAY_IN_LEDGERS;

/// Estados posibles de un depósito de garantía o seña en custodia (escrow).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EstadoDeposito {
    Activo,
    Liberado,
    Reembolsado,
    EnDisputa,
}

/// Datos persistidos de cada acuerdo de depósito.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Deposito {
    pub id: u64,
    pub cliente: Address,
    pub proveedor: Address,
    pub arbitro: Address,
    pub token: Address,
    pub monto: i128,
    pub fecha_limite: u64,
    pub estado: EstadoDeposito,
}

/// Claves de almacenamiento del contrato.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    ContadorId,
    Deposito(u64),
}

/// Errores tipados del contrato de depósito seguro.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    MontoInvalido = 1,
    FechaLimiteInvalida = 2,
    ParticipantesInvalidos = 3,
    DepositoNoEncontrado = 4,
    EstadoInvalido = 5,
    NoAutorizado = 6,
    DesbordamientoId = 7,
}

/// Evento emitido al crear un nuevo depósito en custodia.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DepositoCreado {
    #[topic]
    pub id: u64,
    #[topic]
    pub cliente: Address,
    #[topic]
    pub proveedor: Address,
    pub arbitro: Address,
    pub token: Address,
    pub monto: i128,
    pub fecha_limite: u64,
}

/// Evento emitido cuando el cliente libera los fondos al proveedor.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FondosLiberados {
    #[topic]
    pub id: u64,
    #[topic]
    pub proveedor: Address,
    pub monto: i128,
}

/// Evento emitido cuando los fondos son reembolsados al cliente.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FondosReembolsados {
    #[topic]
    pub id: u64,
    #[topic]
    pub cliente: Address,
    #[topic]
    pub solicitante: Address,
    pub monto: i128,
}

/// Evento emitido cuando el cliente o el proveedor abre una disputa.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputaAbierta {
    #[topic]
    pub id: u64,
    #[topic]
    pub solicitante: Address,
}

/// Evento emitido cuando el árbitro resuelve una disputa.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputaResuelta {
    #[topic]
    pub id: u64,
    #[topic]
    pub arbitro: Address,
    #[topic]
    pub destinatario: Address,
    pub entregar_a_proveedor: bool,
    pub monto: i128,
}

#[contract]
pub struct DepositoSeguroContract;

#[contractimpl]
impl DepositoSeguroContract {
    /// Crea un depósito de garantía/seña. El cliente autentica, transfiere los fondos
    /// al contrato y se guarda el acuerdo con un ID incremental.
    pub fn crear_deposito(
        env: Env,
        cliente: Address,
        proveedor: Address,
        arbitro: Address,
        token: Address,
        monto: i128,
        fecha_limite: u64,
    ) -> Result<u64, Error> {
        cliente.require_auth();

        if monto <= 0 {
            return Err(Error::MontoInvalido);
        }

        if fecha_limite <= env.ledger().timestamp() {
            return Err(Error::FechaLimiteInvalida);
        }

        if cliente == proveedor || cliente == arbitro || proveedor == arbitro {
            return Err(Error::ParticipantesInvalidos);
        }

        let contador_actual: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ContadorId)
            .unwrap_or(0);
        let id = contador_actual
            .checked_add(1)
            .ok_or(Error::DesbordamientoId)?;

        let contrato = env.current_contract_address();
        let token_client = TokenClient::new(&env, &token);
        token_client.transfer(&cliente, &contrato, &monto);

        let deposito = Deposito {
            id,
            cliente: cliente.clone(),
            proveedor: proveedor.clone(),
            arbitro: arbitro.clone(),
            token: token.clone(),
            monto,
            fecha_limite,
            estado: EstadoDeposito::Activo,
        };

        let key = DataKey::Deposito(id);
        env.storage().instance().set(&DataKey::ContadorId, &id);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);

        env.storage().persistent().set(&key, &deposito);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);

        DepositoCreado {
            id,
            cliente,
            proveedor,
            arbitro,
            token,
            monto,
            fecha_limite,
        }
        .publish(&env);

        Ok(id)
    }

    /// Libera los fondos custodiados al proveedor. Solo el cliente puede autorizarlo.
    pub fn liberar_fondos(env: Env, id: u64) -> Result<(), Error> {
        let key = DataKey::Deposito(id);
        let mut deposito: Deposito = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::DepositoNoEncontrado)?;

        if deposito.estado != EstadoDeposito::Activo {
            return Err(Error::EstadoInvalido);
        }

        deposito.cliente.require_auth();

        deposito.estado = EstadoDeposito::Liberado;
        env.storage().persistent().set(&key, &deposito);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);

        let token_client = TokenClient::new(&env, &deposito.token);
        token_client.transfer(
            &env.current_contract_address(),
            &deposito.proveedor,
            &deposito.monto,
        );

        FondosLiberados {
            id,
            proveedor: deposito.proveedor,
            monto: deposito.monto,
        }
        .publish(&env);

        Ok(())
    }

    /// Reembolsa los fondos al cliente.
    /// - Antes o en `fecha_limite`: solo el proveedor puede devolver los fondos.
    /// - Después de `fecha_limite` (`timestamp > fecha_limite`): el cliente puede reclamar el reembolso.
    pub fn reembolsar(env: Env, id: u64) -> Result<(), Error> {
        let key = DataKey::Deposito(id);
        let mut deposito: Deposito = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::DepositoNoEncontrado)?;

        if deposito.estado != EstadoDeposito::Activo {
            return Err(Error::EstadoInvalido);
        }

        let solicitante = if env.ledger().timestamp() > deposito.fecha_limite {
            deposito.cliente.require_auth();
            deposito.cliente.clone()
        } else {
            deposito.proveedor.require_auth();
            deposito.proveedor.clone()
        };

        deposito.estado = EstadoDeposito::Reembolsado;
        env.storage().persistent().set(&key, &deposito);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);

        let token_client = TokenClient::new(&env, &deposito.token);
        token_client.transfer(
            &env.current_contract_address(),
            &deposito.cliente,
            &deposito.monto,
        );

        FondosReembolsados {
            id,
            cliente: deposito.cliente,
            solicitante,
            monto: deposito.monto,
        }
        .publish(&env);

        Ok(())
    }

    /// Cambia el acuerdo a estado `EnDisputa`. Solo el cliente o el proveedor pueden abrirla.
    pub fn abrir_disputa(env: Env, id: u64, solicitante: Address) -> Result<(), Error> {
        let key = DataKey::Deposito(id);
        let mut deposito: Deposito = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::DepositoNoEncontrado)?;

        if deposito.estado != EstadoDeposito::Activo {
            return Err(Error::EstadoInvalido);
        }

        if solicitante != deposito.cliente && solicitante != deposito.proveedor {
            return Err(Error::NoAutorizado);
        }

        solicitante.require_auth();

        deposito.estado = EstadoDeposito::EnDisputa;
        env.storage().persistent().set(&key, &deposito);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);

        DisputaAbierta { id, solicitante }.publish(&env);

        Ok(())
    }

    /// Resuelve un acuerdo en disputa. Solo el árbitro designado puede ejecutar esta función
    /// y enviar los fondos al proveedor (`true`) o de regreso al cliente (`false`).
    pub fn resolver_disputa(
        env: Env,
        id: u64,
        entregar_a_proveedor: bool,
    ) -> Result<(), Error> {
        let key = DataKey::Deposito(id);
        let mut deposito: Deposito = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::DepositoNoEncontrado)?;

        if deposito.estado != EstadoDeposito::EnDisputa {
            return Err(Error::EstadoInvalido);
        }

        deposito.arbitro.require_auth();

        let (nuevo_estado, destinatario) = if entregar_a_proveedor {
            (EstadoDeposito::Liberado, deposito.proveedor.clone())
        } else {
            (EstadoDeposito::Reembolsado, deposito.cliente.clone())
        };

        deposito.estado = nuevo_estado;
        env.storage().persistent().set(&key, &deposito);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);

        let token_client = TokenClient::new(&env, &deposito.token);
        token_client.transfer(
            &env.current_contract_address(),
            &destinatario,
            &deposito.monto,
        );

        DisputaResuelta {
            id,
            arbitro: deposito.arbitro,
            destinatario,
            entregar_a_proveedor,
            monto: deposito.monto,
        }
        .publish(&env);

        Ok(())
    }

    /// Consulta los datos de un depósito por su ID.
    pub fn obtener_deposito(env: Env, id: u64) -> Result<Deposito, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Deposito(id))
            .ok_or(Error::DepositoNoEncontrado)
    }
}

mod test;
