/*
 * Copyright 2023 ByteDance and/or its affiliates.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::sync::Arc;

use anyhow::Context;

use g3_dpi::ProtocolPortMap;
use g3_icap_client::IcapServiceClient;
use g3_types::metrics::MetricsName;

use crate::config::audit::AuditorConfig;
use crate::inspect::tls::TlsInterceptionContext;

mod ops;
pub use ops::load_all;
pub(crate) use ops::reload;

mod registry;
pub(crate) use registry::{get_names, get_or_insert_default};

mod handle;
pub(crate) use handle::AuditHandle;

pub(crate) struct Auditor {
    config: Arc<AuditorConfig>,
    server_tcp_portmap: Arc<ProtocolPortMap>,
    client_tcp_portmap: Arc<ProtocolPortMap>,
    icap_reqmod_service: Option<Arc<IcapServiceClient>>,
    icap_respmod_service: Option<Arc<IcapServiceClient>>,
}

impl Auditor {
    fn new_no_config(name: &MetricsName) -> Arc<Self> {
        let config = AuditorConfig::empty(name);
        Auditor::new_with_config(config)
    }

    fn new_with_config(config: AuditorConfig) -> Arc<Self> {
        let server_tcp_portmap = Arc::new(config.server_tcp_portmap.clone());
        let client_tcp_portmap = Arc::new(config.client_tcp_portmap.clone());
        let icap_reqmod_service = config
            .icap_reqmod_service
            .as_ref()
            .map(|config| Arc::new(IcapServiceClient::new(config.clone())));
        let icap_respmod_service = config
            .icap_respmod_service
            .as_ref()
            .map(|config| Arc::new(IcapServiceClient::new(config.clone())));
        let auditor = Auditor {
            config: Arc::new(config),
            server_tcp_portmap,
            client_tcp_portmap,
            icap_reqmod_service,
            icap_respmod_service,
        };
        Arc::new(auditor)
    }

    fn reload(&self, config: AuditorConfig) -> Arc<Self> {
        let server_tcp_portmap = Arc::new(config.server_tcp_portmap.clone());
        let client_tcp_portmap = Arc::new(config.client_tcp_portmap.clone());
        let icap_reqmod_service = config
            .icap_reqmod_service
            .as_ref()
            .map(|config| Arc::new(IcapServiceClient::new(config.clone())));
        let icap_respmod_service = config
            .icap_respmod_service
            .as_ref()
            .map(|config| Arc::new(IcapServiceClient::new(config.clone())));
        let auditor = Auditor {
            config: Arc::new(config),
            server_tcp_portmap,
            client_tcp_portmap,
            icap_reqmod_service,
            icap_respmod_service,
        };
        Arc::new(auditor)
    }

    pub(crate) fn build_handle(&self) -> anyhow::Result<Arc<AuditHandle>> {
        let mut handle = AuditHandle::new(self);

        if let Some(cert_agent_config) = &self.config.tls_cert_agent {
            let cert_agent = cert_agent_config
                .spawn_cert_agent()
                .context("failed to spawn cert generator task")?;
            let client_config = self
                .config
                .tls_interception_client
                .build()
                .context("failed to build tls client config")?;
            let ctx = TlsInterceptionContext::new(
                cert_agent,
                client_config,
                self.config.tls_stream_dump,
            )?;
            handle.set_tls_interception(ctx);
        }

        Ok(Arc::new(handle))
    }
}
