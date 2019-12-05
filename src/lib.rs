// Copyright 2015-2019 Capital One Services, LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod kvredis;

#[macro_use]
extern crate wascc_codec as codec;

#[macro_use]
extern crate log;

use codec::capabilities::{CapabilityProvider, Dispatcher, NullDispatcher};
use codec::core::CapabilityConfiguration;
use codec::core::OP_CONFIGURE;
use codec::keyvalue;
use keyvalue::*;
use prost::Message;
use redis::Connection;
use redis::RedisResult;
use redis::{self, Commands};
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::sync::RwLock;

const CAPABILITY_ID: &str = "wascc:keyvalue";

capability_provider!(RedisKVProvider, RedisKVProvider::new);

pub struct RedisKVProvider {
    dispatcher: Arc<RwLock<Box<dyn Dispatcher>>>,
    clients: Arc<RwLock<HashMap<String, redis::Client>>>,
}

impl Default for RedisKVProvider {
    fn default() -> Self {
        env_logger::init();

        RedisKVProvider {
            dispatcher: Arc::new(RwLock::new(Box::new(NullDispatcher::new()))),
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl RedisKVProvider {
    pub fn new() -> Self {
        RedisKVProvider::default()
    }

    fn actor_con(&self, actor: &str) -> RedisResult<Connection> {
        let lock = self.clients.read().unwrap();
        lock.get(actor).unwrap().get_connection()
    }

    fn configure(&self, config: CapabilityConfiguration) -> Result<Vec<u8>, Box<dyn Error>> {
        let c = kvredis::initialize_client(config.clone())?;

        self.clients.write().unwrap().insert(config.module, c);
        Ok(vec![])
    }

    fn add(&self, actor: &str, req: AddRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let res: i32 = con.incr(req.key, req.value)?;
        let resp = AddResponse { value: res };

        Ok(bytes(resp))
    }

    fn del(&self, actor: &str, req: DelRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        con.del(&req.key)?;
        let resp = DelResponse { key: req.key };

        Ok(bytes(resp))
    }

    fn get(&self, actor: &str, req: GetRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        if !con.exists(&req.key)? {
            Ok(bytes(GetResponse {
                value: String::from(""),
                exists: false,
            }))
        } else {
            let v: redis::RedisResult<String> = con.get(&req.key);
            Ok(bytes(match v {
                Ok(s) => GetResponse {
                    value: s,
                    exists: true,
                },
                Err(e) => {
                    eprint!("GET for {} failed: {}", &req.key, e);
                    GetResponse {
                        value: "".to_string(),
                        exists: false,
                    }
                }
            }))
        }
    }

    fn list_clear(&self, actor: &str, req: ListClearRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        self.del(actor, DelRequest { key: req.key })
    }

    fn list_range(&self, actor: &str, req: ListRangeRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let result: Vec<String> = con.lrange(req.key, req.start as _, req.stop as _)?;
        Ok(bytes(ListRangeResponse { values: result }))
    }

    fn list_push(&self, actor: &str, req: ListPushRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let result: i32 = con.lpush(req.key, req.value)?;
        Ok(bytes(ListResponse { new_count: result }))
    }

    fn set(&self, actor: &str, req: SetRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        con.set(req.key, &req.value)?;
        Ok(bytes(SetResponse {
            value: req.value.clone(),
        }))
    }

    fn list_del_item(
        &self,
        actor: &str,
        req: ListDelItemRequest,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let result: i32 = con.lrem(req.key, 0, &req.value)?;
        Ok(bytes(ListResponse { new_count: result }))
    }

    fn set_add(&self, actor: &str, req: SetAddRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let result: i32 = con.sadd(req.key, &req.value)?;
        Ok(bytes(SetOperationResponse { new_count: result }))
    }

    fn set_remove(&self, actor: &str, req: SetRemoveRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let result: i32 = con.srem(req.key, &req.value)?;
        Ok(bytes(SetOperationResponse { new_count: result }))
    }

    fn set_union(&self, actor: &str, req: SetUnionRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let result: Vec<String> = con.sunion(req.keys)?;
        Ok(bytes(SetQueryResponse { values: result }))
    }

    fn set_intersect(
        &self,
        actor: &str,
        req: SetIntersectionRequest,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let result: Vec<String> = con.sinter(req.keys)?;
        Ok(bytes(SetQueryResponse { values: result }))
    }

    fn set_query(&self, actor: &str, req: SetQueryRequest) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let result: Vec<String> = con.smembers(req.key)?;
        Ok(bytes(SetQueryResponse { values: result }))
    }

    fn exists(&self, actor: &str, req: KeyExistsQuery) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut con = self.actor_con(actor)?;
        let result: bool = con.exists(req.key)?;
        Ok(bytes(GetResponse {
            value: "".to_string(),
            exists: result,
        }))
    }
}

fn bytes(msg: impl prost::Message) -> Vec<u8> {
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    buf
}

impl CapabilityProvider for RedisKVProvider {
    fn capability_id(&self) -> &'static str {
        CAPABILITY_ID
    }

    fn configure_dispatch(&self, dispatcher: Box<dyn Dispatcher>) -> Result<(), Box<dyn Error>> {
        trace!("Dispatcher received.");

        let mut lock = self.dispatcher.write().unwrap();
        *lock = dispatcher;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "waSCC Default Key-Value Provider (Redis)"
    }

    fn handle_call(&self, actor: &str, op: &str, msg: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
        info!("Received host call, operation - {}", op);

        match op {
            OP_CONFIGURE if actor == "system" => {
                self.configure(CapabilityConfiguration::decode(msg).unwrap())
            }
            keyvalue::OP_ADD => self.add(actor, AddRequest::decode(msg).unwrap()),
            keyvalue::OP_DEL => self.del(actor, DelRequest::decode(msg).unwrap()),
            keyvalue::OP_GET => self.get(actor, GetRequest::decode(msg).unwrap()),
            keyvalue::OP_CLEAR => self.list_clear(actor, ListClearRequest::decode(msg).unwrap()),
            keyvalue::OP_RANGE => self.list_range(actor, ListRangeRequest::decode(msg).unwrap()),
            keyvalue::OP_PUSH => self.list_push(actor, ListPushRequest::decode(msg).unwrap()),
            keyvalue::OP_SET => self.set(actor, SetRequest::decode(msg).unwrap()),
            keyvalue::OP_LIST_DEL => {
                self.list_del_item(actor, ListDelItemRequest::decode(msg).unwrap())
            }
            keyvalue::OP_SET_ADD => self.set_add(actor, SetAddRequest::decode(msg).unwrap()),
            keyvalue::OP_SET_REMOVE => {
                self.set_remove(actor, SetRemoveRequest::decode(msg).unwrap())
            }
            keyvalue::OP_SET_UNION => self.set_union(actor, SetUnionRequest::decode(msg).unwrap()),
            keyvalue::OP_SET_INTERSECT => {
                self.set_intersect(actor, SetIntersectionRequest::decode(msg).unwrap())
            }
            keyvalue::OP_SET_QUERY => self.set_query(actor, SetQueryRequest::decode(msg).unwrap()),
            keyvalue::OP_KEY_EXISTS => self.exists(actor, KeyExistsQuery::decode(msg).unwrap()),
            _ => Err("bad dispatch".into()),
        }
    }
}