use wanix_fs::OpenOptions;
use wanix_protocol::{P9OpenIntent, p9_open_flags};

/// Encodes `wanix-fs` [`OpenOptions`] into a 9P2000.L open-flags word.
///
/// This is the client-side inverse of the server's flag decoding: it lowers a
/// Wanix open request into the `Tlopen`/`Tlcreate` flags a 9P client must send.
/// Append is not part of [`OpenOptions`], so it is never set here.
#[must_use]
pub fn open_flags_for(options: OpenOptions) -> u32 {
    p9_open_flags(P9OpenIntent {
        read: options.read,
        write: options.write,
        create: options.create,
        truncate: options.truncate,
        append: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wanix_protocol::{O_CREAT, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY, p9_open_intent};

    #[test]
    fn read_write_options_map_to_rdwr() {
        assert_eq!(open_flags_for(OpenOptions::read_write()), O_RDWR);
    }

    #[test]
    fn read_only_options_map_to_rdonly() {
        assert_eq!(open_flags_for(OpenOptions::read()), O_RDONLY);
    }

    #[test]
    fn write_create_truncate_set_expected_bits() {
        let options = OpenOptions {
            read: false,
            write: true,
            create: true,
            truncate: true,
        };
        assert_eq!(open_flags_for(options), O_WRONLY | O_CREAT | O_TRUNC);
    }

    #[test]
    fn client_flags_round_trip_through_server_intent() {
        let options = OpenOptions {
            read: true,
            write: true,
            create: true,
            truncate: true,
        };
        let intent = p9_open_intent(open_flags_for(options));
        assert_eq!(intent.read, options.read);
        assert_eq!(intent.write, options.write);
        assert_eq!(intent.create, options.create);
        assert_eq!(intent.truncate, options.truncate);
    }
}
