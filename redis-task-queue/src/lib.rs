use serde::{Deserialize, Serialize};

use log::*;
use base64::{Engine as _, engine::general_purpose};
use uuid::Uuid;

use tokio::time;
use std::time::Duration;

use axum::{
    extract::{Path, Json, Query, State},
   
    http::StatusCode,
    response::{IntoResponse,Response}
};

use redis::{self, aio::MultiplexedConnection};

#[derive(Debug, Serialize, Deserialize)]
pub struct Job{
    pub project: Option<String>,
    pub msg: Option<String>,
    pub queue: Option<String>,
    pub job_id: Option<String>,
    pub delay: i64,
    pub ttl: i64,
    pub tries: i32,
    pub data: Option<String>
}

pub async fn queue_len(State(con):State<MultiplexedConnection>, 
        Path((project,queue_id)): Path<(String, String)>) 
        -> Response{

    let  result : Option<i32> = redis::cmd("LLEN")
            .arg(project + "|" + queue_id.as_str())
            .query_async(&mut con.clone()).await.unwrap();

    (StatusCode::OK, format!("{}", result.unwrap())).into_response()
}

pub async fn delete_job(State(con):State<MultiplexedConnection>, 
Path((project, queue_id, job_id)): Path<(String, String, String)>)
    -> impl IntoResponse{
        debug!("job_id:{}",job_id);

        //unlink is del but not blocking https://database.guide/4-ways-to-delete-a-key-in-redis/
        let result : Option<i32> = redis::cmd("UNLINK")
            .arg(job_id)
            .query_async(&mut con.clone()).await.unwrap();

        StatusCode::OK
}


pub async fn insert_job(State(con):State<MultiplexedConnection>, 
    Path((project,queue)): Path<(String, String)>,
    Query(mut job): Query<Job>,
    data: String
    ) -> impl IntoResponse{
        job.project = Some(project);
        job.queue = Some(queue);
        job.job_id = Some(Uuid::new_v4().to_string());
        job.msg = Some("new job".to_string());

        let enc_data = general_purpose::STANDARD.encode(data);

        job.data = Some(enc_data);

        let now = chrono::Utc::now().timestamp();

        //use timestamp for easy process.
        job.ttl = now + &job.ttl;
        job.delay = now + &job.delay;

        debug!("job: {:?}", &job);

    process_job(con, &job).await.unwrap_or_default();

    //returned job ttl/delay is relative
    job.ttl -= now;
    job.delay -= now;

    (StatusCode::CREATED, Json(job))
}
#[derive(Deserialize)]
pub struct PopJobParam{
    ttr: i64
}
pub async fn pop_job(State(con):State<MultiplexedConnection>, 
    Path((project,queue)): Path<(String, String)>,
    Query(param): Query<PopJobParam>
    )
    -> Response{

        debug!("ttr:{:?}", param.ttr);

        loop {

            let result: Option<String> = redis::cmd("LPOP")
                .arg(project.clone() + "|" + queue.as_str())
                .query_async(&mut con.clone()).await.unwrap();

            debug!("LPOP result{:?}", &result);

            //no more job in the queue
            if result.is_none(){
                return StatusCode::NOT_FOUND.into_response();
            }

            //get the job data
            let job: Option<String> = redis::cmd("GET").
                        arg(result)
                        .query_async(&mut con.clone()).await.unwrap();
            debug!("job: {:?}", job);

            //job is deleted, may be by other worker, try another.
            if job.is_none(){
                continue;
            }

            let mut job : Job = serde_json::from_str(job.unwrap().as_str()).unwrap();
            let now = chrono::Utc::now().timestamp();

            //It's Over
            if job.ttl < now {
                continue;
            }

            //put the job to delay queue for next try
            if job.tries > 0 && job.ttl > now {
                job.tries -= 1;
                job.delay = now + param.ttr;
                process_job(con, &job).await.unwrap();
            }

            //returned job ttl/delay is relative
            job.ttl -= now;
            job.delay -= now;

            return Json(job).into_response()
        }
}

async fn process_job(con: MultiplexedConnection, job : &Job)->Result<(), ()>{
    //insert a job
    //insert jobid to queue if delay < 1
    debug!("Process Job");

    let result: Option<String> = redis::cmd("SET")
        .arg(&job.job_id)
        .arg(serde_json::to_string(&job).unwrap())
        .query_async(&mut con.clone()).await.unwrap();

    let now = chrono::Utc::now().timestamp();

    if job.delay <= now{
        
        let result : Option<i32>= redis::cmd("RPUSH")
            .arg(job.project.clone().unwrap() + "|" + job.queue.clone().unwrap().as_str())
            .arg(&job.job_id)
            .query_async(&mut con.clone()).await.unwrap();

        debug!("RPUSH {:?}", result);

    }else{ //insert jobid to delay_queue (sort set by timestamp) if delay > now
        let result : Option<i32> = redis::cmd("ZADD")
        .arg("delay_job")
        .arg(job.delay) 
        .arg(&job.job_id)
        .query_async(&mut con.clone()).await.unwrap();

        debug!("ZADD {:?}", result);

    }

    Ok(())
}

pub async fn process_delay_jobs(){
    //debug!("Spawn a job!");
    let mut interval = time::interval(Duration::from_secs(1));

    let redis_server_addr=std::env::var("REDIS_SERVER").unwrap();
    let redis_client = redis::Client::open(redis_server_addr).unwrap();

    let mycon = redis_client.get_multiplexed_tokio_connection().await.unwrap();

    loop {
        interval.tick().await;
        //debug!("tick!");

        let ts = chrono::Utc::now().timestamp();

        //check delayed job (include first delay and out of exe time)
        let result : Vec<String> = redis::cmd("zrangebyscore")
        .arg("delay_job")
        .arg("-inf")
        .arg(&ts)
        .query_async(&mut mycon.clone()).await.unwrap();

        //debug!("zrangebyscore {:?}", result);

        let job_id_iter = result.iter();

        for job_id in job_id_iter{
            let job: Option<String> = redis::cmd("GET").
                arg(job_id)
                .query_async(&mut mycon.clone()).await.unwrap();
            debug!("job: {:?}", job);

            let result: Option<i32> = redis::cmd("ZREM")
            .arg("delay_job")
            .arg(job_id)
            .query_async(&mut mycon.clone()).await.unwrap();

            match &job{
                Some(t) => (),
                None => continue
            }
            
            let t : Job = serde_json::from_str(job.unwrap().as_str()).unwrap();

            let _: Option<i32>  = redis::cmd("RPUSH")
                .arg(t.project.clone().unwrap() + "|" + t.queue.unwrap().as_str())
                .arg(t.job_id)
                .query_async(&mut mycon.clone()).await.unwrap();
           
        }

        //remove job
        
    }
}