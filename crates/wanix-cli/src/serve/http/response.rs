pub(in crate::serve) struct StaticResponse {
    pub(in crate::serve) status: HttpStatus,
    pub(in crate::serve) content_type: &'static str,
    pub(in crate::serve) headers: Vec<(&'static str, String)>,
    pub(in crate::serve) body: Vec<u8>,
}

impl StaticResponse {
    pub(in crate::serve) fn plain(status: HttpStatus, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(in crate::serve) fn with_header(
        mut self,
        name: &'static str,
        value: impl Into<String>,
    ) -> Self {
        let value = value.into().replace(['\r', '\n'], "");
        self.headers.push((name, value));
        self
    }

    pub(in crate::serve) fn encode(&self) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {}\r\n\
             Content-Length: {}\r\n\
             Content-Type: {}\r\n",
            self.status.status_line(),
            self.body.len(),
            self.content_type
        )
        .into_bytes();
        for (name, value) in &self.headers {
            response.extend_from_slice(name.as_bytes());
            response.extend_from_slice(b": ");
            response.extend_from_slice(value.as_bytes());
            response.extend_from_slice(b"\r\n");
        }
        response.extend_from_slice(
            b"Cross-Origin-Opener-Policy: same-origin\r\n\
              Cross-Origin-Embedder-Policy: require-corp\r\n\
              Access-Control-Allow-Origin: *\r\n\
              Connection: close\r\n\
              \r\n",
        );
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
    InternalServerError,
    ServiceUnavailable,
}

impl HttpStatus {
    const REASONS: [&'static str; 9] = [
        "ok",
        "bad request",
        "conflict",
        "forbidden",
        "not found",
        "method not allowed",
        "not implemented",
        "internal server error",
        "service unavailable",
    ];
    const STATUS_LINES: [&'static str; 9] = [
        "200 OK",
        "400 Bad Request",
        "409 Conflict",
        "403 Forbidden",
        "404 Not Found",
        "405 Method Not Allowed",
        "501 Not Implemented",
        "500 Internal Server Error",
        "503 Service Unavailable",
    ];

    pub(in crate::serve) fn status_line(self) -> &'static str {
        Self::STATUS_LINES[self as usize]
    }

    pub(in crate::serve) fn reason(self) -> &'static str {
        Self::REASONS[self as usize]
    }
}
