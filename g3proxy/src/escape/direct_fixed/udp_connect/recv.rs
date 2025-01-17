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

use std::task::{ready, Context, Poll};

use g3_io_ext::{AsyncUdpRecv, UdpCopyRemoteError, UdpCopyRemoteRecv};
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
))]
use g3_io_ext::{RecvMsgBuf, RecvMsgHdr, UdpCopyPacket};

pub(crate) struct DirectUdpConnectRemoteRecv<T> {
    inner: T,
}

impl<T> DirectUdpConnectRemoteRecv<T>
where
    T: AsyncUdpRecv,
{
    pub(crate) fn new(recv: T) -> Self {
        DirectUdpConnectRemoteRecv { inner: recv }
    }
}

impl<T> UdpCopyRemoteRecv for DirectUdpConnectRemoteRecv<T>
where
    T: AsyncUdpRecv,
{
    fn max_hdr_len(&self) -> usize {
        0
    }

    fn poll_recv_packet(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<(usize, usize), UdpCopyRemoteError>> {
        let nr = ready!(self.inner.poll_recv(cx, buf)).map_err(UdpCopyRemoteError::RecvFailed)?;
        Poll::Ready(Ok((0, nr)))
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    fn poll_recv_packets(
        &mut self,
        cx: &mut Context<'_>,
        packets: &mut [UdpCopyPacket],
    ) -> Poll<Result<usize, UdpCopyRemoteError>> {
        let mut meta = vec![RecvMsgHdr::default(); packets.len()];
        let mut bufs: Vec<_> = packets
            .iter_mut()
            .map(|p| RecvMsgBuf::new(p.buf_mut()))
            .collect();

        let count = ready!(self.inner.poll_batch_recvmsg(cx, &mut bufs, &mut meta))
            .map_err(UdpCopyRemoteError::RecvFailed)?;

        for (p, m) in packets.iter_mut().take(count).zip(meta) {
            p.set_offset(0);
            p.set_length(m.len);
        }

        Poll::Ready(Ok(count))
    }
}
