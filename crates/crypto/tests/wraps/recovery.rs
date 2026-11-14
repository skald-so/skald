//! The recovery key's HPKE key pair: `DeriveKeyPair(recovery key)` of DHKEM(X25519, HKDF-SHA256)
//! (RFC 9180, §7.1.3), `docs/protocol/membership.md`, "Account".

use crate::common::{hex, hex32};
use crate::fixtures::{RECOVERY, recovery_key};
use crate::rfc9180::{AAD, CT, ENC, INFO, PT};

/// RFC 9180, Appendix A: every `DeriveKeyPair` of DHKEM(X25519, HKDF-SHA256) in the vectors of
/// A.1, A.2 and A.7, as `ikm`, `sk` and `pk`.
const VECTORS: [[&str; 3]; 30] = [
    // A.1.1
    [
        "7268600d403fce431561aef583ee1613527cff655c1343f29812e66706df3234",
        "52c4a758a802cd8b936eceea314432798d5baf2d7e9235dc084ab1b9cfa2f736",
        "37fda3567bdbd628e88668c3c8d7e97d1d1253b6d4ea6d44c150f741f1bf4431",
    ],
    [
        "6db9df30aa07dd42ee5e8181afdb977e538f5e1fec8a06223f33f7013e525037",
        "4612c550263fc8ad58375df3f557aac531d26850903e55a9f23f21d8534e8ac8",
        "3948cfe0ad1ddb695d780e59077195da6c56506b027329794ab02bca80815c4d",
    ],
    // A.1.2
    [
        "78628c354e46f3e169bd231be7b2ff1c77aa302460a26dbfa15515684c00130b",
        "463426a9ffb42bb17dbe6044b9abd1d4e4d95f9041cef0e99d7824eef2b6f588",
        "0ad0950d9fb9588e59690b74f1237ecdf1d775cd60be2eca57af5a4b0471c91b",
    ],
    [
        "d4a09d09f575fef425905d2ab396c1449141463f698f8efdb7accfaff8995098",
        "c5eb01eb457fe6c6f57577c5413b931550a162c71a03ac8d196babbd4e5ce0fd",
        "9fed7e8c17387560e92cc6462a68049657246a09bfa8ade7aefe589672016366",
    ],
    // A.1.3
    [
        "6e6d8f200ea2fb20c30b003a8b4f433d2f4ed4c2658d5bc8ce2fef718059c9f7",
        "ff4442ef24fbc3c1ff86375b0be1e77e88a0de1e79b30896d73411c5ff4c3518",
        "23fb952571a14a25e3d678140cd0e5eb47a0961bb18afcf85896e5453c312e76",
    ],
    [
        "f1d4a30a4cef8d6d4e3b016e6fd3799ea057db4f345472ed302a67ce1c20cdec",
        "fdea67cf831f1ca98d8e27b1f6abeb5b7745e9d35348b80fa407ff6958f9137e",
        "1632d5c2f71c2b38d0a8fcc359355200caa8b1ffdf28618080466c909cb69b2e",
    ],
    [
        "94b020ce91d73fca4649006c7e7329a67b40c55e9e93cc907d282bbbff386f58",
        "dc4a146313cce60a278a5323d321f051c5707e9c45ba21a3479fecdf76fc69dd",
        "8b0c70873dc5aecb7f9ee4e62406a397b350e57012be45cf53b7105ae731790b",
    ],
    // A.1.4
    [
        "4303619085a20ebcf18edd22782952b8a7161e1dbae6e46e143a52a96127cf84",
        "14de82a5897b613616a00c39b87429df35bc2b426bcfd73febcb45e903490768",
        "820818d3c23993492cc5623ab437a48a0a7ca3e9639c140fe1e33811eb844b7c",
    ],
    [
        "4b16221f3b269a88e207270b5e1de28cb01f847841b344b8314d6a622fe5ee90",
        "cb29a95649dc5656c2d054c1aa0d3df0493155e9d5da6d7e344ed8b6a64a9423",
        "1d11a3cd247ae48e901939659bd4d79b6b959e1f3e7d66663fbc9412dd4e0976",
    ],
    [
        "62f77dcf5df0dd7eac54eac9f654f426d4161ec850cc65c54f8b65d2e0b4e345",
        "fc1c87d2f3832adb178b431fce2ac77c7ca2fd680f3406c77b5ecdf818b119f4",
        "2bfb2eb18fcad1af0e4f99142a1c474ae74e21b9425fc5c589382c69b50cc57e",
    ],
    // A.2.1
    [
        "909a9b35d3dc4713a5e72a4da274b55d3d3821a37e5d099e74a647db583a904b",
        "f4ec9b33b792c372c1d2c2063507b684ef925b8c75a42dbcbf57d63ccd381600",
        "1afa08d3dec047a643885163f1180476fa7ddb54c6a8029ea33f95796bf2ac4a",
    ],
    [
        "1ac01f181fdf9f352797655161c58b75c656a6cc2716dcb66372da835542e1df",
        "8057991eef8f1f1af18f4a9491d16a1ce333f695d4db8e38da75975c4478e0fb",
        "4310ee97d88cc1f088a5576c77ab0cf5c3ac797f3d95139c6c84b5429c59662a",
    ],
    // A.2.2
    [
        "35706a0b09fb26fb45c39c2f5079c709c7cf98e43afa973f14d88ece7e29c2e3",
        "0c35fdf49df7aa01cd330049332c40411ebba36e0c718ebc3edf5845795f6321",
        "2261299c3f40a9afc133b969a97f05e95be2c514e54f3de26cbe5644ac735b04",
    ],
    [
        "26b923eade72941c8a85b09986cdfa3f1296852261adedc52d58d2930269812b",
        "77d114e0212be51cb1d76fa99dd41cfd4d0166b08caa09074430a6c59ef17879",
        "13640af826b722fc04feaa4de2f28fbd5ecc03623b317834e7ff4120dbe73062",
    ],
    // A.2.3
    [
        "938d3daa5a8904540bc24f48ae90eed3f4f7f11839560597b55e7c9598c996c0",
        "c94619e1af28971c8fa7957192b7e62a71ca2dcdde0a7cc4a8a9e741d600ab13",
        "f7674cc8cd7baa5872d1f33dbaffe3314239f6197ddf5ded1746760bfc847e0e",
    ],
    [
        "64835d5ee64aa7aad57c6f2e4f758f7696617f8829e70bc9ac7a5ef95d1c756c",
        "3ca22a6d1cda1bb9480949ec5329d3bf0b080ca4c45879c95eddb55c70b80b82",
        "1a478716d63cb2e16786ee93004486dc151e988b34b475043d3e0175bdb01c44",
    ],
    [
        "9d8f94537d5a3ddef71234c0baedfad4ca6861634d0b94c3007fed557ad17df6",
        "2def0cb58ffcf83d1062dd085c8aceca7f4c0c3fd05912d847b61f3e54121f05",
        "f0f4f9e96c54aeed3f323de8534fffd7e0577e4ce269896716bcb95643c8712b",
    ],
    // A.2.4
    [
        "49d6eac8c6c558c953a0a252929a818745bb08cd3d29e15f9f5db5eb2e7d4b84",
        "5e6dd73e82b856339572b7245d3cbb073a7561c0bee52873490e305cbb710410",
        "656a2e00dc9990fd189e6e473459392df556e9a2758754a09db3f51179a3fc02",
    ],
    [
        "f3304ddcf15848488271f12b75ecaf72301faabf6ad283654a14c398832eb184",
        "7b36a42822e75bf3362dfabbe474b3016236408becb83b859a6909e22803cb0c",
        "a5099431c35c491ec62ca91df1525d6349cb8aa170c51f9581f8627be6334851",
    ],
    [
        "20ade1d5203de1aadfb261c4700b6432e260d0d317be6ebbb8d7fffb1f86ad9d",
        "90761c5b0a7ef0985ed66687ad708b921d9803d51637c8d1cb72d03ed0f64418",
        "3ac5bd4dd66ff9f2740bef0d6ccb66daa77bff7849d7895182b07fb74d087c45",
    ],
    // A.7.1
    [
        "55bc245ee4efda25d38f2d54d5bb6665291b99f8108a8c4b686c2b14893ea5d9",
        "095182b502f1f91f63ba584c7c3ec473d617b8b4c2cec3fad5af7fa6748165ed",
        "e5e8f9bfff6c2f29791fc351d2c25ce1299aa5eaca78a757c0b4fb4bcd830918",
    ],
    [
        "683ae0da1d22181e74ed2e503ebf82840deb1d5e872cade20f4b458d99783e31",
        "33d196c830a12f9ac65d6e565a590d80f04ee9b19c83c87f2c170d972a812848",
        "194141ca6c3c3beb4792cd97ba0ea1faff09d98435012345766ee33aae2d7664",
    ],
    // A.7.2
    [
        "c51211a8799f6b8a0021fcba673d9c4067a98ebc6794232e5b06cb9febcbbdf5",
        "1d72396121a6a826549776ef1a9d2f3a2907fc6a38902fa4e401afdb0392e627",
        "d3805a97cbcd5f08babd21221d3e6b362a700572d14f9bbeb94ec078d051ae3d",
    ],
    [
        "5e0516b1b29c0e13386529da16525210c796f7d647c37eac118023a6aa9eb89a",
        "98f304d4ecb312689690b113973c61ffe0aa7c13f2fbe365e48f3ed09e5a6a0c",
        "d53af36ea5f58f8868bb4a1333ed4cc47e7a63b0040eb54c77b9c8ec456da824",
    ],
    // A.7.3
    [
        "43b078912a54b591a7b09b16ce89a1955a9dd60b29fb611e044260046e8b061b",
        "83d3f217071bbf600ba6f081f6e4005d27b97c8001f55cb5ff6ea3bbea1d9295",
        "5ac1671a55c5c3875a8afe74664aa8bc68830be9ded0c5f633cd96400e8b5c05",
    ],
    [
        "fc9407ae72ed614901ebf44257fb540f617284b5361cfecd620bafc4aba36f73",
        "ed88cda0e91ca5da64b6ad7fc34a10f096fa92f0b9ceff9d2c55124304ed8b4a",
        "ffd7ac24694cb17939d95feb7c4c6539bb31621deb9b96d715a64abdd9d14b10",
    ],
    [
        "2ff4c37a17b2e54046a076bf5fea9c3d59250d54d0dc8572bc5f7c046307040c",
        "c85f136e06d72d28314f0e34b10aadc8d297e9d71d45a5662c2b7c3b9f9f9405",
        "89eb1feae431159a5250c5186f72a15962c8d0debd20a8389d8b6e4996e14306",
    ],
    // A.7.4
    [
        "94efae91e96811a3a49fd1b20eb0344d68ead6ac01922c2360779aa172487f40",
        "a2b43f5c67d0d560ee04de0122c765ea5165e328410844db97f74595761bbb81",
        "81cbf4bd7eee97dd0b600252a1c964ea186846252abb340be47087cc78f3d87c",
    ],
    [
        "4dfde6fadfe5cb50fced4034e84e6d3a104aa4bf2971360032c1c0580e286663",
        "c4962a7f97d773a47bdf40db4b01dc6a56797c9e0deaab45f4ea3aa9b1d72904",
        "f47cd9d6993d2e2234eb122b425accfb486ee80f89607b087094e9f413253c2d",
    ],
    [
        "26c12fef8d71d13bbbf08ce8157a283d5e67ecf0f345366b0e90341911110f1b",
        "6175b2830c5743dff5b7568a7e20edb1fe477fb0487ca21d6433365be90234d0",
        "29a5bf3867a6128bbdf8e070abe7fe70ca5e07b629eba5819af73810ee20112f",
    ],
];

/// `ikmR` of [`crate::rfc9180`], whose key pair its encryption is to.
const IKM_R: &str = "6db9df30aa07dd42ee5e8181afdb977e538f5e1fec8a06223f33f7013e525037";

/// An X25519 private key clamped as RFC 7748, §5, decodes a scalar.
fn clamped(mut secret: [u8; 32]) -> [u8; 32] {
    secret[0] &= 248;
    secret[31] &= 127;
    secret[31] |= 64;
    secret
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_recovery_key_pair_matches_the_rfc_derive_key_pair_vectors() {
    let mut unclamped = 0;
    for [ikm, sk, pk] in VECTORS {
        let key = recovery_key(&hex32(ikm));
        assert_eq!(key.public_key(), hex32(pk), "pk of {ikm}");
        // hpke clamps the X25519 private keys it serializes; the vectors' are not clamped.
        assert_eq!(*key.to_bytes(), clamped(hex32(sk)), "sk of {ikm}, clamped");
        unclamped += usize::from(clamped(hex32(sk)) != hex32(sk));
    }
    assert!(
        unclamped > 0,
        "the vectors hold private keys that clamping changes"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_key_pair_of_the_rfc_base_vectors_ikm_opens_its_ciphertext() {
    let key = recovery_key(&hex32(IKM_R));
    let opened = key
        .open(&hex32(ENC), &hex(INFO), &hex(AAD), &hex(CT))
        .expect("the vector opens with the derived key");
    assert_eq!(opened.as_slice(), hex(PT).as_slice());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_recovery_key_always_gives_the_same_key_pair_and_another_one_another() {
    let key = recovery_key(&RECOVERY);
    assert_eq!(recovery_key(&RECOVERY).public_key(), key.public_key());
    assert_eq!(*recovery_key(&RECOVERY).to_bytes(), *key.to_bytes());
    assert_ne!(
        *key.to_bytes(),
        RECOVERY,
        "the private key is derived, not the recovery key"
    );
    let mut other = RECOVERY;
    other[31] ^= 1;
    assert_ne!(recovery_key(&other).public_key(), key.public_key());
}
