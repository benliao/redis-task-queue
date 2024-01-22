use axum::{
    async_trait,
    extract::{FromRequestParts, Path},
    http::{request::Parts, StatusCode},
    middleware,
    routing::{delete, get, put},
    RequestPartsExt, Router,
};

use serde::{Deserialize, Serialize};

use log::*;

use dotenvy::dotenv;

use redis_task_queue::{delete_job, insert_job, pop_job, process_delay_jobs, queue_len};

use tokio::task;

use std::collections::HashMap;

struct RequireAuth;

#[derive(Debug, Serialize, Deserialize)]
struct Token {
    project: String,
    token: String,
}

#[macro_use]
extern crate lazy_static;

lazy_static!(
    //static ref X_TOKEN: String = std::env::var("QUEUE_X_TOKEN").unwrap();
    static ref TOKENS: HashMap<String, String>= read_tokens();
);

fn read_tokens() -> HashMap<String, String> {
    let tokens: Vec<Token> =
        serde_json::from_str::<_>(&std::fs::read_to_string("api-token.json").unwrap()).unwrap();

    let mut hash_tokens = HashMap::new();

    for token in tokens.iter() {
        hash_tokens.insert(token.project.clone(), token.token.clone());
    }

    hash_tokens
}

#[derive(Debug, Serialize, Deserialize)]
struct ReqPath {
    project: String,
}

#[async_trait]
impl<S> FromRequestParts<S> for RequireAuth
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let project = parts
            .extract::<Path<ReqPath>>()
            .await
            .map(|Path(path)| path)
            .unwrap_or(ReqPath {
                project: "defalut".to_string(),
            });

        let auth_header = parts.headers.get("X-token");

        if let Some(value) = auth_header {
            if TOKENS.contains_key(&project.project) &&
                &*TOKENS[&project.project] == value {
                    return Ok(Self);
                
            }
        }

        Err(StatusCode::UNAUTHORIZED)
    }
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    env_logger::init();

    debug!("Tokens: {:?}", *TOKENS);

    let redis_server_addr = std::env::var("REDIS_SERVER").unwrap();
    let redis_client = redis::Client::open(redis_server_addr).unwrap();
    let con = redis_client
        .get_multiplexed_tokio_connection()
        .await
        .unwrap();

    let join = task::spawn(process_delay_jobs());

    let app = Router::new()
        .route("/queue-api/:project/:queue", put(insert_job).get(pop_job))
        .route("/queue-api/:project/:queue/job/:job_id", delete(delete_job))
        .route("/queue-api/:project/:queue/size", get(queue_len))
        .with_state(con)
        .layer(middleware::from_extractor::<RequireAuth>());

    let addr = std::env::var("API_BIND_ADDR")
        .unwrap()
        .parse()
        .expect("not valid address");
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    let _ = join.await;
}
