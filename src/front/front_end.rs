// Copyright (c) 2019, Arm Limited, All Rights Reserved
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may
// not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//          http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Entry point for IPC data into the service
//!
//! The front end handler accepts streams of data that it can use to read requests,
//! pass them to the rest of the service and write the responses back.
use crate::authenticators::Authenticate;
use crate::back::dispatcher::Dispatcher;
use derivative::Derivative;
use log::{error, info};
use parsec_interface::requests::AuthType;
use parsec_interface::requests::ResponseStatus;
use parsec_interface::requests::{Request, Response};
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};
use std::io::{Read, Write};

/// Read and verify request from IPC stream
///
/// Service component that serializes requests and deserializes responses
/// from/to the stream provided by the listener.
///
/// Requests are passed forward to the `Dispatcher`.
#[derive(Derivative)]
#[derivative(Debug)]
pub struct FrontEndHandler {
    dispatcher: Dispatcher,
    // Send and Sync are required for Arc<FrontEndHandler> to be Send.
    #[derivative(Debug = "ignore")]
    authenticators: HashMap<AuthType, Box<dyn Authenticate + Send + Sync>>,
    /// Value used to limit the size of the request body to be that can be accepted by the service.
    body_len_limit: usize,
}

impl FrontEndHandler {
    /// Handle new connections on the underlying IPC mechanism.
    ///
    /// Unmarshalls a request from the stream, passes it to the dispatcher and marshalls
    /// the response back onto the stream.
    ///
    /// If an error occurs during (un)marshalling, no operation will be performed and the
    /// method will return.
    pub fn handle_request<T: Read + Write>(&self, mut stream: T) {
        // Read bytes from stream
        // De-Serialise bytes into a request
        let request = match Request::read_from_stream(&mut stream, self.body_len_limit) {
            Ok(request) => request,
            Err(status) => {
                error!("Failed to read request; status: {}", status);

                let response = Response::from_status(status);
                if let Err(status) = response.write_to_stream(&mut stream) {
                    error!("Failed to write response; status: {}", status);
                }
                return;
            }
        };
        // Check if the request was sent without authentication
        let response = if AuthType::NoAuth == request.header.auth_type {
            self.dispatcher.dispatch_request(request, None)
        // Otherwise find an authenticator that is capable to authenticate the request
        } else if let Some(authenticator) = self.authenticators.get(&request.header.auth_type) {
            // Authenticate the request
            match authenticator.authenticate(&request.auth) {
                // Send the request to the dispatcher
                // Get a response back
                Ok(app_name) => self.dispatcher.dispatch_request(request, Some(app_name)),
                Err(status) => Response::from_request_header(request.header, status),
            }
        } else {
            Response::from_request_header(
                request.header,
                ResponseStatus::AuthenticatorNotRegistered,
            )
        };

        // Serialise the responso into bytes
        // Write bytes to stream
        match response.write_to_stream(&mut stream) {
            Ok(_) => info!("Request handled successfully"),
            Err(err) => error!("Failed to send response; error: {}", err),
        }
    }
}

/// Builder for `FrontEndHandler`
#[derive(Default, Derivative)]
#[derivative(Debug)]
pub struct FrontEndHandlerBuilder {
    dispatcher: Option<Dispatcher>,
    #[derivative(Debug = "ignore")]
    authenticators: Option<HashMap<AuthType, Box<dyn Authenticate + Send + Sync>>>,
    body_len_limit: Option<usize>,
}

impl FrontEndHandlerBuilder {
    pub fn new() -> Self {
        FrontEndHandlerBuilder {
            dispatcher: None,
            authenticators: None,
            body_len_limit: None,
        }
    }

    pub fn with_dispatcher(mut self, dispatcher: Dispatcher) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    pub fn with_authenticator(
        mut self,
        auth_type: AuthType,
        authenticator: Box<dyn Authenticate + Send + Sync>,
    ) -> Self {
        match &mut self.authenticators {
            Some(authenticators) => {
                let _ = authenticators.insert(auth_type, authenticator);
            }
            None => {
                let mut map = HashMap::new();
                let _ = map.insert(auth_type, authenticator);
                self.authenticators = Some(map);
            }
        };

        self
    }

    pub fn with_body_len_limit(mut self, body_len_limit: usize) -> Self {
        self.body_len_limit = Some(body_len_limit);
        self
    }

    pub fn build(self) -> Result<FrontEndHandler> {
        Ok(FrontEndHandler {
            dispatcher: self
                .dispatcher
                .ok_or_else(|| Error::new(ErrorKind::InvalidData, "dispatcher is missing"))?,
            authenticators: self
                .authenticators
                .ok_or_else(|| Error::new(ErrorKind::InvalidData, "authenticators is missing"))?,
            body_len_limit: self
                .body_len_limit
                .ok_or_else(|| Error::new(ErrorKind::InvalidData, "body_len_limit is missing"))?,
        })
    }
}
