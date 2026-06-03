use anyhow::{Error, Result};

#[derive(Default)]
pub(super) struct CleanupScope {
    error: Option<Error>,
}

impl CleanupScope {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn record<E>(&mut self, cleanup: std::result::Result<(), E>)
    where
        E: Into<Error>,
    {
        if self.error.is_none()
            && let Err(err) = cleanup
        {
            self.error = Some(err.into());
        }
    }

    pub(super) fn finish<T, E>(self, result: std::result::Result<T, E>) -> Result<T>
    where
        E: Into<Error>,
    {
        finish_with_cleanup(result, self.error)
    }

    pub(super) fn into_error(self) -> Option<Error> {
        self.error
    }
}

pub(super) fn finish_with_cleanup<T, E>(
    result: std::result::Result<T, E>,
    cleanup_error: Option<Error>,
) -> Result<T>
where
    E: Into<Error>,
{
    match (result, cleanup_error) {
        (Ok(value), None) => Ok(value),
        (Ok(_), Some(err)) => Err(err),
        (Err(err), _) => Err(err.into()),
    }
}
