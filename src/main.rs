use std::sync::Arc;
use serde_json::Value;
use serde_json::json;
use ton_client::ClientConfig;
use ton_client::ClientContext;
use ton_client::abi::Abi;
use ton_client::abi::AbiContract;
use ton_client::abi::DeploySet;
use ton_client::abi::ParamsOfEncodeMessage;
use ton_client::abi::CallSet;
use ton_client::abi::Signer;
use ton_client::crypto::KeyPair;
use ton_client::processing::ParamsOfProcessMessage;
use ton_client::processing::ProcessingEvent;

const NETWORK_URL: &str = "net.ton.dev";
const WORKCHAIN: i32 = 0;
const CONFIG_PATH: &str = "config.json";

#[tokio::main]
async fn main() {
    let config = match get_config() {
        Ok(v) => v,
        Err(e) => return println!("Cannot load config: {}", e),
    };
    let initial_data = match get_initial_data(config["initial_data"].as_str()) {
        Ok(v) => v,
        Err(e) => return println!("Cannot load initial data: {}", e),
    };
    match deploy(
        config["parameters"].as_str(),
        initial_data,
    ).await {
        Ok(_) => println!("Ok"),
        Err(e) => println!("Fail: {}", e),
    };
}

fn get_initial_data(path: Option<&str>) -> Result<Value, String> {
    if let Some(p) = path {
        return serde_json::from_slice(std::fs::read(p).map_err(|e| e.to_string())?.as_ref()).map_err(|e| e.to_string())
    }
    Err("initial data file is not defined. Supply path in the `config.json`.".to_string())
}

fn get_config() -> Result<Value, String> {
    Ok(serde_json::from_str(&std::fs::read_to_string(CONFIG_PATH).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?)
}

async fn deploy(
    params: Option<&str>,
    initial_data: Value,
) -> Result<(), String> {
    let abi = Some(load_abi(initial_data["abi_path"].as_str())?);
    let params = Some(load_params(params.unwrap())?);
    deploy_contract(
        initial_data["code_base64"].as_str().unwrap(),
        &abi.unwrap(),
        &params.unwrap(),
        initial_data["public_key"].as_str(),
        initial_data["secret_key"].as_str(),
    ).await
}

fn load_abi(abi_path: Option<&str>) -> Result<String, String> {
    abi_path.map(|s| s.to_string())
       .ok_or("ABI file is not defined. Supply it in the config.json.".to_string())
}

fn load_params(params: &str) -> Result<String, String> {
    Ok(if params.find('{').is_none() {
        std::fs::read_to_string(params)
            .map_err(|e| format!("failed to load params from file: {}", e))?
    } else {
        params.to_string()
    })
}

fn create_client_verbose() -> Result<Arc<ClientContext>, String> {
    Ok(Arc::new(ClientContext::new(ClientConfig {
        network: ton_client::net::NetworkConfig {
            server_address: Some(NETWORK_URL.to_owned()),
            message_processing_timeout: 30000,
            ..Default::default()
        },
        ..Default::default()
    }).map_err(|e| format!("failed to create tonclient: {}", e))?))
    // create_client(workchain_id, is_json, endpoints)
}

async fn process_message(
    ton: Arc<ClientContext>,
    msg: ParamsOfEncodeMessage,
    is_json: bool,
) -> Result<serde_json::Value, String> {
    let callback = |event| { async move {
        if let ProcessingEvent::DidSend { shard_block_id: _, message_id, message: _ } = event {
            println!("MessageId: {}", message_id)
        }
    }};
    let res = if !is_json {
        ton_client::processing::process_message(
            ton.clone(),
            ParamsOfProcessMessage {
                message_encode_params: msg.clone(),
                send_events: true,
                ..Default::default()
            },
            callback,
        ).await
    } else {
        ton_client::processing::process_message(
            ton.clone(),
            ParamsOfProcessMessage {
                message_encode_params: msg.clone(),
                send_events: true,
                ..Default::default()
            },
            |_| { async move {} },
        ).await
    }.map_err(|e| format!("{:#}", e))?;

    Ok(res.decoded.and_then(|d| d.output).unwrap_or(json!({})))
}

async fn deploy_contract(
    code_base64: &str,
    abi: &str,
    params: &str,
    public_key: Option<&str>,
    secret_key: Option<&str>,
) -> Result<(), String> {
    let ton = create_client_verbose()?;
    let (msg, addr) = prepare_deploy_message(
        code_base64,
        abi,
        params,
        public_key,
        secret_key,
    ).await?;

    process_message(ton.clone(), msg, false).await?;

    println!("Transaction succeeded.");
    println!("Contract deployed at address: {}", addr);
    Ok(())
}

fn get_context() -> Result<Arc<ClientContext>, String> {
    Ok(Arc::new(ClientContext::new(ClientConfig::default())
        .map_err(|e| format!("failed to create client context: {}", e))?))
}

async fn calc_acc_address(
    tvc_base64: String,
    pubkey: Option<String>,
    abi: Abi,
) -> Result<String, String> {
    let ton = get_context();
    let dset = DeploySet {
        tvc: tvc_base64,
        workchain_id: Some(WORKCHAIN),
        ..Default::default()
    };
    let result = ton_client::abi::encode_message(
        ton.clone()?,
        ParamsOfEncodeMessage {
            abi,
            deploy_set: Some(dset),
            signer: if pubkey.is_some() {
                Signer::External {
                    public_key: pubkey.unwrap(),
                }
            } else {
                Signer::None
            },
            ..Default::default()
        },
    )
    .await
    .map_err(|e| format!("cannot generate address: {}", e))?;
    Ok(result.address)
}

fn load_keypair(public_key: Option<&str>, secret_key: Option<&str>) -> Result<Option<KeyPair>, String> {
    if let (Some(p), Some(s)) = (public_key, secret_key) {
        let keys_str = format!(r#"{{
            "public": "{}",
            "secret": "{}"
        }}"#, p, s);
        return Ok(serde_json::from_str(&keys_str)
            .map_err(|e| format!("failed to load keypair: {}", e))?);
    }
    Ok(None)
}

async fn prepare_deploy_message(
    code_base64: &str,
    abi_path: &str,
    params: &str,
    public_key: Option<&str>,
    secret_key: Option<&str>,
) -> Result<(ParamsOfEncodeMessage, String), String> {
    let abi_str = std::fs::read_to_string(abi_path)
        .map_err(|e| format!("failed to read ABI file: {}", e))?;
    let abi = Abi::Contract(
        serde_json::from_str::<AbiContract>(&abi_str)
            .map_err(|e| format!("ABI is not a valid json: {}", e))?,
    );
    let keypair = load_keypair(
        public_key,
        secret_key,
    )?;
    let addr = calc_acc_address(
        code_base64.to_string(),
        keypair.as_ref().map(|k| k.public.clone()),
        abi.clone()
    ).await?;
    let params = serde_json::from_str(params)
        .map_err(|e| format!("function arguments is not a json: {}", e))?;

    Ok((ParamsOfEncodeMessage {
        abi,
        address: Some(addr.clone()),
        deploy_set: Some(DeploySet {
            tvc: code_base64.to_string(),
            workchain_id: Some(WORKCHAIN),
            ..Default::default()
        }),
        call_set: CallSet::some_with_function_and_input("constructor", params),
        signer: Signer::Keys{ keys: keypair.unwrap() },
        ..Default::default()
    }, addr))
}
