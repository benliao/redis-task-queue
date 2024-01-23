use serde::{Deserialize, Serialize};

use base64::{engine::general_purpose, Engine as _};
use log::*;
use uuid::Uuid;

use std::time::Duration;
use tokio::time;

use axum::{
    extract::{Json, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use redis::{self, aio::MultiplexedConnection};

#[derive(Debug, Serialize, Deserialize)]
pub struct Task {
    pub project: Option<String>,
    pub msg: Option<String>,
    pub queue: Option<String>,
    pub task_id: Option<String>,
    pub delay: i64,
    pub ttl: i64,
    pub tries: i32,
    pub data: Option<String>,
}

pub async fn queue_len(
    State(con): State<MultiplexedConnection>,
    Path((project, queue_id)): Path<(String, String)>,
) -> Response {
    let result: Option<i32> = redis::cmd("LLEN")
        .arg(project + "|" + queue_id.as_str())
        .query_async(&mut con.clone())
        .await
        .unwrap();

    (StatusCode::OK, format!("{}", result.unwrap())).into_response()
}

pub async fn delete_task(
    State(con): State<MultiplexedConnection>,
    Path((_project, _queue_id, task_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    debug!("task_id:{}", task_id);

    //unlink is del but not blocking https://database.guide/4-ways-to-delete-a-key-in-redis/
    let result: Option<i32> = redis::cmd("UNLINK")
        .arg(task_id)
        .query_async(&mut con.clone())
        .await
        .unwrap();

    match result { 
        Some(i) => {
            if i > 0 {
                StatusCode::OK
            }else {
                StatusCode::NOT_FOUND
            }
        },
        None => StatusCode::NOT_FOUND
    }
}

pub async fn insert_task(
    State(con): State<MultiplexedConnection>,
    Path((project, queue)): Path<(String, String)>,
    Query(mut task): Query<Task>,
    data: String,
) -> impl IntoResponse {
    task.project = Some(project);
    task.queue = Some(queue);
    task.task_id = Some(Uuid::new_v4().to_string());
    task.msg = Some("new task".to_string());

    let enc_data = general_purpose::STANDARD.encode(data);

    task.data = Some(enc_data);

    let now = chrono::Utc::now().timestamp();

    //use timestamp for easy process.
    task.ttl += now;
    task.delay += now;

    debug!("task: {:?}", &task);

    process_task(con, &task).await.unwrap_or_default();

    //returned task ttl/delay is relative
    task.ttl -= now;
    task.delay -= now;

    (StatusCode::CREATED, Json(task))
}
#[derive(Deserialize)]
pub struct PoptaskParam {
    ttr: i64,
}
pub async fn pop_task(
    State(con): State<MultiplexedConnection>,
    Path((project, queue)): Path<(String, String)>,
    Query(param): Query<PoptaskParam>,
) -> Response {
    debug!("ttr:{:?}", param.ttr);

    loop {
        let result: Option<String> = redis::cmd("LPOP")
            .arg(project.clone() + "|" + queue.as_str())
            .query_async(&mut con.clone())
            .await
            .unwrap();

        debug!("LPOP result{:?}", &result);

        //no more task in the queue
        if result.is_none() {
            return StatusCode::NOT_FOUND.into_response();
        }

        //get the task data
        let task: Option<String> = redis::cmd("GET")
            .arg(result)
            .query_async(&mut con.clone())
            .await
            .unwrap();
        debug!("task: {:?}", task);

        //task is deleted, may be by other worker, try another.
        if task.is_none() {
            continue;
        }

        let mut task: Task = serde_json::from_str(task.unwrap().as_str()).unwrap();
        let now = chrono::Utc::now().timestamp();

        //It's Over
        if task.ttl < now {
            continue;
        }

        //put the task to delay queue for next try
        if task.tries > 0 && task.ttl > now {
            task.tries -= 1;
            task.delay = now + param.ttr;
            process_task(con, &task).await.unwrap();
        }

        //returned task ttl/delay is relative
        task.ttl -= now;
        task.delay -= now;

        return Json(task).into_response();
    }
}

async fn process_task(con: MultiplexedConnection, task: &Task) -> Result<(), ()> {
    //insert a task
    //insert taskid to queue if delay < 1
    debug!("Process task");

    let _: Option<String> = redis::cmd("SET")
        .arg(&task.task_id)
        .arg(serde_json::to_string(&task).unwrap())
        .query_async(&mut con.clone())
        .await
        .unwrap();

    let now = chrono::Utc::now().timestamp();

    if task.delay <= now {
        let result: Option<i32> = redis::cmd("RPUSH")
            .arg(task.project.clone().unwrap() + "|" + task.queue.clone().unwrap().as_str())
            .arg(&task.task_id)
            .query_async(&mut con.clone())
            .await
            .unwrap();

        debug!("RPUSH {:?}", result);
    } else {
        //insert taskid to delay_queue (sort set by timestamp) if delay > now
        let result: Option<i32> = redis::cmd("ZADD")
            .arg("delay_task")
            .arg(task.delay)
            .arg(&task.task_id)
            .query_async(&mut con.clone())
            .await
            .unwrap();

        debug!("ZADD {:?}", result);
    }

    Ok(())
}

pub async fn process_delay_tasks() {
    //debug!("Spawn a task!");
    let mut interval = time::interval(Duration::from_secs(1));

    let redis_server_addr = std::env::var("REDIS_SERVER").unwrap();
    let redis_client = redis::Client::open(redis_server_addr).unwrap();

    let mycon = redis_client
        .get_multiplexed_tokio_connection()
        .await
        .unwrap();

    loop {
        interval.tick().await;
        //debug!("tick!");

        let ts = chrono::Utc::now().timestamp();

        //check delayed task (include first delay and out of exe time)
        let result: Vec<String> = redis::cmd("zrangebyscore")
            .arg("delay_task")
            .arg("-inf")
            .arg(ts)
            .query_async(&mut mycon.clone())
            .await
            .unwrap();

        //debug!("zrangebyscore {:?}", result);

        let task_id_iter = result.iter();

        for task_id in task_id_iter {
            let task: Option<String> = redis::cmd("GET")
                .arg(task_id)
                .query_async(&mut mycon.clone())
                .await
                .unwrap();
            debug!("task: {:?}", task);

            let _: Option<i32> = redis::cmd("ZREM")
                .arg("delay_task")
                .arg(task_id)
                .query_async(&mut mycon.clone())
                .await
                .unwrap();

            match &task {
                Some(_) => (),
                None => continue,
            }

            let t: Task = serde_json::from_str(task.unwrap().as_str()).unwrap();

            let _: Option<i32> = redis::cmd("RPUSH")
                .arg(t.project.clone().unwrap() + "|" + t.queue.unwrap().as_str())
                .arg(t.task_id)
                .query_async(&mut mycon.clone())
                .await
                .unwrap();
        }

        //remove task
    }
}
