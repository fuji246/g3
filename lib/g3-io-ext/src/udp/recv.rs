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
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::task::{ready, Context, Poll};
use std::time::Duration;

use futures_util::FutureExt;
use tokio::time::{Instant, Sleep};

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
))]
use super::{RecvMsgBuf, RecvMsgHdr};
use crate::limit::{DatagramLimitInfo, DatagramLimitResult};
use crate::ArcLimitedRecvStats;

pub trait AsyncUdpRecv {
    fn poll_recv_from(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>>;

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>>;

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    fn poll_batch_recvmsg(
        &mut self,
        cx: &mut Context<'_>,
        bufs: &mut [RecvMsgBuf<'_>],
        meta: &mut [RecvMsgHdr],
    ) -> Poll<io::Result<usize>>;
}

pub struct LimitedUdpRecv<T> {
    inner: T,
    delay: Pin<Box<Sleep>>,
    started: Instant,
    limit: DatagramLimitInfo,
    stats: ArcLimitedRecvStats,
}

impl<T: AsyncUdpRecv> LimitedUdpRecv<T> {
    pub fn new(
        inner: T,
        shift_millis: u8,
        max_packets: usize,
        max_bytes: usize,
        stats: ArcLimitedRecvStats,
    ) -> Self {
        LimitedUdpRecv {
            inner,
            delay: Box::pin(tokio::time::sleep(Duration::from_millis(0))),
            started: Instant::now(),
            limit: DatagramLimitInfo::new(shift_millis, max_packets, max_bytes),
            stats,
        }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    pub fn reset_stats(&mut self, stats: ArcLimitedRecvStats) {
        self.stats = stats;
    }
}

impl<T> AsyncUdpRecv for LimitedUdpRecv<T>
where
    T: AsyncUdpRecv + Send,
{
    fn poll_recv_from(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr)>> {
        if self.limit.is_set() {
            let dur_millis = self.started.elapsed().as_millis() as u64;
            match self.limit.check_packet(dur_millis, buf.len()) {
                DatagramLimitResult::Advance(_) => {
                    let (nr, addr) = ready!(self.inner.poll_recv_from(cx, buf))?;
                    self.limit.set_advance(1, nr);
                    self.stats.add_recv_packet();
                    self.stats.add_recv_bytes(nr);
                    Poll::Ready(Ok((nr, addr)))
                }
                DatagramLimitResult::DelayFor(ms) => {
                    self.delay
                        .as_mut()
                        .reset(self.started + Duration::from_millis(dur_millis + ms));
                    self.delay
                        .poll_unpin(cx)
                        .map(|_| Ok((0, SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))))
                }
            }
        } else {
            let (nr, addr) = ready!(self.inner.poll_recv_from(cx, buf))?;
            self.stats.add_recv_packet();
            self.stats.add_recv_bytes(nr);
            Poll::Ready(Ok((nr, addr)))
        }
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        if self.limit.is_set() {
            let dur_millis = self.started.elapsed().as_millis() as u64;
            match self.limit.check_packet(dur_millis, buf.len()) {
                DatagramLimitResult::Advance(_) => {
                    let nr = ready!(self.inner.poll_recv(cx, buf))?;
                    self.limit.set_advance(1, nr);
                    self.stats.add_recv_packet();
                    self.stats.add_recv_bytes(nr);
                    Poll::Ready(Ok(nr))
                }
                DatagramLimitResult::DelayFor(ms) => {
                    self.delay
                        .as_mut()
                        .reset(self.started + Duration::from_millis(dur_millis + ms));
                    self.delay.poll_unpin(cx).map(|_| Ok(0))
                }
            }
        } else {
            let nr = ready!(self.inner.poll_recv(cx, buf))?;
            self.stats.add_recv_packet();
            self.stats.add_recv_bytes(nr);
            Poll::Ready(Ok(nr))
        }
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    fn poll_batch_recvmsg(
        &mut self,
        cx: &mut Context<'_>,
        bufs: &mut [RecvMsgBuf<'_>],
        meta: &mut [RecvMsgHdr],
    ) -> Poll<io::Result<usize>> {
        if self.limit.is_set() {
            let dur_millis = self.started.elapsed().as_millis() as u64;
            match self.limit.check_packets(dur_millis, bufs) {
                DatagramLimitResult::Advance(n) => {
                    let count = ready!(self.inner.poll_batch_recvmsg(
                        cx,
                        &mut bufs[0..n],
                        &mut meta[0..n]
                    ))?;
                    let len = meta.iter().take(count).map(|h| h.len).sum();
                    self.limit.set_advance(count, len);
                    self.stats.add_recv_packets(count);
                    self.stats.add_recv_bytes(len);
                    Poll::Ready(Ok(count))
                }
                DatagramLimitResult::DelayFor(ms) => {
                    self.delay
                        .as_mut()
                        .reset(self.started + Duration::from_millis(dur_millis + ms));
                    self.delay.poll_unpin(cx).map(|_| Ok(0))
                }
            }
        } else {
            let count = ready!(self.inner.poll_batch_recvmsg(cx, bufs, meta))?;
            self.stats.add_recv_packets(count);
            self.stats
                .add_recv_bytes(meta.iter().take(count).map(|h| h.len).sum());
            Poll::Ready(Ok(count))
        }
    }
}
