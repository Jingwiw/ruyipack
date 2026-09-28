// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Loopback HTTP/TLS exercises the production transport.

use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub struct Server {
    pub url: String,
    pub calls: Arc<Mutex<Vec<String>>>,
    ca: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

trait Connection: Read + Write {}
impl<T: Read + Write> Connection for T {}

impl Server {
    pub fn new(tls: bool, handle: impl Fn(&str) -> Vec<u8> + Send + 'static) -> Self {
        let ca = tempfile::tempdir().unwrap();
        std::fs::write(ca.path().join("ca.pem"), CA).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "{}://{}",
            if tls { "https" } else { "http" },
            listener.local_addr().unwrap()
        );
        listener.set_nonblocking(true).unwrap();
        let config = Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from_pem_slice(CERT.as_bytes()).unwrap()],
                    PrivateKeyDer::from_pem_slice(KEY.as_bytes()).unwrap(),
                )
                .unwrap(),
        );
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let received = calls.clone();
        let thread = thread::spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                let socket = match listener.accept() {
                    Ok((socket, _)) => socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("accept: {e}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut stream: Box<dyn Connection> = if tls {
                    Box::new(StreamOwned::new(
                        ServerConnection::new(config.clone()).unwrap(),
                        socket,
                    ))
                } else {
                    Box::new(socket)
                };
                let mut request = String::new();
                let mut reader = BufReader::new(&mut stream);
                // Failed certificate validation closes the connection before an HTTP request.
                if reader.read_line(&mut request).is_err() || request.is_empty() {
                    continue;
                }
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err()
                        || header == "\r\n"
                        || header.is_empty()
                    {
                        break;
                    }
                }
                let path = request.split_whitespace().nth(1).unwrap();
                received.lock().unwrap().push(path.to_owned());
                let mut response = handle(path);
                // One request per socket: never pool a connection this fixture will close.
                let status_end = response.windows(2).position(|w| w == b"\r\n").unwrap() + 2;
                response.splice(
                    status_end..status_end,
                    b"Connection: close\r\n".iter().copied(),
                );
                let _ = stream.write_all(&response);
                let _ = stream.flush();
            }
        });
        Self {
            url,
            calls,
            ca,
            stop,
            thread: Some(thread),
        }
    }

    pub fn command(&self) -> Command {
        let mut command = super::support::command();
        command
            .env("PATH", self.ca.path())
            .env("SSL_CERT_FILE", self.ca.path().join("ca.pem"));
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env_remove(name);
        }
        command
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

pub fn response(body: &[u8]) -> Vec<u8> {
    let mut result =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    result.extend_from_slice(body);
    result
}

// Test-only key and certificate chain, valid until 2126; never used by the product.
const CA: &str = r#"-----BEGIN CERTIFICATE-----
MIIDKTCCAhGgAwIBAgIUI57UjPmWrNW3SamZzc3MiOVEOqUwDQYJKoZIhvcNAQEL
BQAwGzEZMBcGA1UEAwwQUnV5aVBhY2sgdGVzdCBDQTAgFw0yNjA5MjgwNjQ5NDBa
GA8yMTI2MDkwNDA2NDk0MFowGzEZMBcGA1UEAwwQUnV5aVBhY2sgdGVzdCBDQTCC
ASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBALiZuKkOrF/yr4jFV8NppfxJ
dU0IjQYg2Q+oN4h3pPOUG1RXYLbbOTGEhBm7Xo1z9qWt1Ij1LPX1eU3WffBzsx+l
T9/pe59W2oQ+phg5FhQYIw9t2HxQpXx0veQ9AvKJ9JjtI9ztAPIKNJ/Hw8sFJf27
4oSLG04pHbncyKzS0MJZEQNk+Y5kz9ehgSDMJesnB+b83d7RPTQ+6klIikIS/Gfs
PqXre6BeHcaTUNwaMSRU7w0IxI3RBcff9RlQg+I6dGz2F3hZ6P8ouhVGjTKH/Ec7
WoNuVacGiUEslS/Xk5gIFflcqnDVUH6AsWQeMRZWiYzu5w5t2V9lpoS4ukWUPzUC
AwEAAaNjMGEwHQYDVR0OBBYEFBZY78Gube1XNlf413VZGzXQu0pnMB8GA1UdIwQY
MBaAFBZY78Gube1XNlf413VZGzXQu0pnMA8GA1UdEwEB/wQFMAMBAf8wDgYDVR0P
AQH/BAQDAgEGMA0GCSqGSIb3DQEBCwUAA4IBAQC14tIQO/+euqorGf0Mz5LprwoI
L1W879RkBHq6vjNZzL9qW+A7nK12gk8Ozy7Did6llQ/mxjIr1KpEGabOeg8gzJ04
ELQlrHeEwg7Wb46QiK3auJ+TTNXMx9A13PpIJwn7O+Utz7TYT1B4grWpccFpQdaO
h1nBdqhlyNSAJSdzxOaiBYUmhwMQOsTDoPFsXHVeyktDNyGQ2h2wnl8FO4dEfxbo
cjrgzksgBvG6Gi4z7BWtzpVn5CnopEUWetbAgOvuGhtdsP30tuD0za7we+XiB8U5
G+JkJAMo+0jlPEONLW4yGgixCTOwEd9u6jCQAQ3hKG2d4R3ohgPAWjtFHMAj
-----END CERTIFICATE-----
"#;
const CERT: &str = r#"-----BEGIN CERTIFICATE-----
MIIDUjCCAjqgAwIBAgIUHsm5/ishsF0T3WOVsPX0MOHUzkYwDQYJKoZIhvcNAQEL
BQAwGzEZMBcGA1UEAwwQUnV5aVBhY2sgdGVzdCBDQTAgFw0yNjA5MjgwNjQ5NDBa
GA8yMTI2MDkwNDA2NDk0MFowFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjANBgkq
hkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAzR+7jnxpfyLeEg1P9FNdnbJlnnTd95IT
Jaa5TPYp4abyYPXZ2eDNKNO5uzQKef+nk4FKl54TfPdynkSL0/4jO75qsEB4o+yc
iMCu0sa8Ya2bSC087SpPBLzXrixkcozKJnsZvYrHQUqSlgjxVQ7YO2ucIgw9w29I
KLJTpB2egcryUt1jZY7z/8188/j69TjxwDYT4dgKaIQ/jKm2SKPg7wdklcwjwrf4
mv9pgWDHsQeSyT/v32TfD39xVnVZgUY4tbgK/I6kJb1Ppkp7mQGQGnUfPKog+WtE
hYdKeXj+eouRVNviHn/jCPLyA3Rbv9A/2DXJqLDi+Vb6eiwcbDllUwIDAQABo4GS
MIGPMBoGA1UdEQQTMBGCCWxvY2FsaG9zdIcEfwAAATAMBgNVHRMBAf8EAjAAMA4G
A1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAdBgNVHQ4EFgQU8Npd
YlnOkqqLeohDAJ17CpAb0wYwHwYDVR0jBBgwFoAUFljvwa5t7Vc2V/jXdVkbNdC7
SmcwDQYJKoZIhvcNAQELBQADggEBACgvlRbyH4/jogFYCAj+s0pX7fd/lG1NuAnf
z5D4S5p2KWnfc0dTUptUKahvqoPixNPDTKvpENXWjHNifgRKcZdSfja3rXOpI4LS
llHEH9d14GnxzumW6AoOxxQfL0SLAgx6S6VwEaK6uCADom36ANFodnjE+9l3Tj4X
+nfsAcxwk7zzLHuavdvwBDmxbcLGBfIEGOhGnyI3yHQcElHWyIwgNRCYqEq6vkFo
kQ8xIlJ/dshcJfJiZpGRciFCoBEMK3cqctyEypGNhKaoE92t5DhYAzC0lEqAIHn1
JNm9JDLs3TLzvUoTvituJajnOo/XSRhKoxqbLodSNCU7M/zx84E=
-----END CERTIFICATE-----
"#;
const KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDNH7uOfGl/It4S
DU/0U12dsmWedN33khMlprlM9inhpvJg9dnZ4M0o07m7NAp5/6eTgUqXnhN893Ke
RIvT/iM7vmqwQHij7JyIwK7SxrxhrZtILTztKk8EvNeuLGRyjMomexm9isdBSpKW
CPFVDtg7a5wiDD3Db0goslOkHZ6ByvJS3WNljvP/zXzz+Pr1OPHANhPh2ApohD+M
qbZIo+DvB2SVzCPCt/ia/2mBYMexB5LJP+/fZN8Pf3FWdVmBRji1uAr8jqQlvU+m
SnuZAZAadR88qiD5a0SFh0p5eP56i5FU2+Ief+MI8vIDdFu/0D/YNcmosOL5Vvp6
LBxsOWVTAgMBAAECggEAHdZY1PAZ9Glg/iU7lSGvQ2oYyATd6M4xxM9Msvo3u2Aq
b5ozdzLxBNhPcwCt6XMbsCQlcoqG8S2ZWZZE9LBJ9b3MDRlDsyyO1IGarIRGELtN
FCGodCMsXVcO1IIWlmcOXKyZYO3X8BJl4jcoy6OeJn6PtpArR8tfJrRR4FSCowRc
b/HvEc8Sj9wX5avXgVKWk0rKks83kor3AURNPOL+wczh3sKBNf5GLXthM2XgzwEC
TBG2rLzpZXhBuTXGkFpXWTRDji5db4csbbiHX+U6RjEwMNQd+AcN13Du4FwbuiCQ
gjcwsLhtVoChUk7SvSwtfM0Z/Tt97aUMOENVj0VYaQKBgQD6V8B5gq6/GITNGso3
zqBv/i2dMzyRb4HpoSQsc6g4RLuoK1bWTQXhb1ZUDFmQNFMFepaXsZ6SZ7WbZWHn
RoSPlT1rxN7Vuxym+8gQKIe1ykK9eyKQvFdS2pKDR0YpHGj03VIjctFW18GVV55Y
m2T11q+QVCwPRhxG7lcZmUSY3QKBgQDRwmMcHDt5Rpks1Wc7UYGMoxZZzDulnFI9
GcGSEjQ//lT7jzYdnocXr/SiBHQogXaeTcZoTlU2mRTqH5nGw0IjfH3S9BH+wbnS
ge8fWbADTuU4RBX54PDgR6fi5tHOrLqyFyPWCT0BycBx/rsMrPKP7z9qVHPFMfsx
8Kd/hKb77wKBgA8mTFGOJQEPfMnkuyQRbwgX+66tsRakBtqak9PU0/NDxY9xv/mM
A7UWxcSkUq81W2jTeFWJvCzj9cuHoRsb213NDEB/U3Tfs+YvCnZf3YaUzOEmmHrl
yusKqx8iqw5F19wpoJTgl+aHfAGLodt+2+c8rLcxQNFTztZECiVUbyBJAoGATNcx
B3MwNlUud8YVcx2An8x+u5adoyWI2uk8iA4zJd49s4nbAS65vmuu6ktHYi9LDOLg
9AT+Imohx0KcSrvs1qMcVNMkZHcDY6JFvu5UFGIqhloq0sccdozJa82yvkt4eRUR
A6+OscD+xsPSMeqJUUELsiAN6QdORhUqxwQJR/ECgYEAtzNn8eZTXsw1DELLGHi6
wI7WHoyVOLZaE83ZL0gkyfK/yPFIHw+hwi/J01KKP2cNHUvOGcg3EGIFLDqVRpi+
xCVDlGQcOVsINi+UKyx7AbvQ4wi0OPc/YklaHa3lBc0cDK8loYQ5pi2tFM61ocj6
IXNAzgxWEmUX1SmQwc/Udbg=
-----END PRIVATE KEY-----
"#;
