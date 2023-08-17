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

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use pin_project::pin_project;
use tokio::io::AsyncWrite;

use g3_http::server::HttpProxyClientRequest;
use g3_io_ext::LimitedWriter;
use g3_types::net::UpstreamAddr;

use super::ProxySocks5EscaperStats;
use crate::auth::UserUpstreamTrafficStats;
use crate::module::http_forward::{
    send_req_header_to_origin, ArcHttpForwardTaskRemoteStats, HttpForwardRemoteWrapperStats,
    HttpForwardTaskRemoteWrapperStats, HttpForwardWrite,
};
use crate::serve::ServerTaskNotes;

#[pin_project]
pub(super) struct ProxySocks5HttpForwardWriter<W: AsyncWrite> {
    #[pin]
    inner: W,
    escaper_stats: Option<Arc<ProxySocks5EscaperStats>>,
}

impl<W> ProxySocks5HttpForwardWriter<W>
where
    W: AsyncWrite,
{
    pub(super) fn new(ups_w: W, escaper_stats: Option<Arc<ProxySocks5EscaperStats>>) -> Self {
        ProxySocks5HttpForwardWriter {
            inner: ups_w,
            escaper_stats,
        }
    }
}

impl<W> AsyncWrite for ProxySocks5HttpForwardWriter<W>
where
    W: AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let this = self.project();
        this.inner.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let this = self.project();
        this.inner.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let this = self.project();
        this.inner.poll_shutdown(cx)
    }
}

#[async_trait]
impl<W> HttpForwardWrite for ProxySocks5HttpForwardWriter<LimitedWriter<W>>
where
    W: AsyncWrite + Send + Unpin,
{
    fn prepare_new(&mut self, _task_notes: &ServerTaskNotes, _upstream: &UpstreamAddr) {}

    fn update_stats(
        &mut self,
        task_stats: &ArcHttpForwardTaskRemoteStats,
        user_stats: Vec<Arc<UserUpstreamTrafficStats>>,
    ) {
        if let Some(escaper_stats) = &self.escaper_stats {
            let mut wrapper_stats = HttpForwardRemoteWrapperStats::new(escaper_stats, task_stats);
            wrapper_stats.push_user_io_stats(user_stats);
            self.inner.reset_stats(Arc::new(wrapper_stats) as _);
        } else {
            let mut wrapper_stats = HttpForwardTaskRemoteWrapperStats::new(Arc::clone(task_stats));
            wrapper_stats.push_user_io_stats(user_stats);
            self.inner.reset_stats(Arc::new(wrapper_stats) as _);
        }
    }

    async fn send_request_header<'a>(
        &'a mut self,
        req: &'a HttpProxyClientRequest,
    ) -> io::Result<()> {
        send_req_header_to_origin(&mut self.inner, req).await
    }
}
