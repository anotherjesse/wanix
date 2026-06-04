pub(in crate::serve) struct StaticResponse {
    pub(in crate::serve) status: HttpStatus,
    pub(in crate::serve) content_type: &'static str,
    pub(in crate::serve) body: Vec<u8>,
}

impl StaticResponse {
    pub(in crate::serve) fn plain(status: HttpStatus, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    pub(in crate::serve) fn encode(&self) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {}\r\n\
             Content-Length: {}\r\n\
             Content-Type: {}\r\n\
             Cross-Origin-Opener-Policy: same-origin\r\n\
             Cross-Origin-Embedder-Policy: require-corp\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Connection: close\r\n\
             \r\n",
            self.status.status_line(),
            self.body.len(),
            self.content_type
        )
        .into_bytes();
        response.extend_from_slice(&self.body);
        response
    }
}

#[derive(Copy, Clone)]
#[repr(usize)]
pub(in crate::serve) enum HttpStatus {
    Ok,
    BadRequest,
    Conflict,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    NotImplemented,
}

impl HttpStatus {
    const REASONS: [&'static str; 7] = [
        "ok",
        "bad request",
        "conflict",
        "forbidden",
        "not found",
        "method not allowed",
        "not implemented",
    ];
    const STATUS_LINES: [&'static str; 7] = [
        "200 OK",
        "400 Bad Request",
        "409 Conflict",
        "403 Forbidden",
        "404 Not Found",
        "405 Method Not Allowed",
        "501 Not Implemented",
    ];

    pub(in crate::serve) fn status_line(self) -> &'static str {
        Self::STATUS_LINES[self as usize]
    }

    pub(in crate::serve) fn reason(self) -> &'static str {
        Self::REASONS[self as usize]
    }
}
