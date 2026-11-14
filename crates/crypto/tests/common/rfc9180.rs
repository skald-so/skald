//! RFC 9180, Appendix A.1.1: base mode of DHKEM(X25519, HKDF-SHA256), HKDF-SHA256 and AES-128-GCM,
//! and its encryption at sequence number 0, as hex digits.

pub const ENC: &str = "37fda3567bdbd628e88668c3c8d7e97d1d1253b6d4ea6d44c150f741f1bf4431";
pub const INFO: &str = "4f6465206f6e2061204772656369616e2055726e";
pub const PT: &str = "4265617574792069732074727574682c20747275746820626561757479";
pub const AAD: &str = "436f756e742d30";
pub const CT: &str = "f938558b5d72f1a23810b4be2ab4f84331acc02fc97babc53a52ae8218a355a9
6d8770ac83d07bea87e13c512a";
