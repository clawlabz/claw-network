/// TDD test suite for claw-contract CLI.
///
/// These tests are written FIRST (RED phase) before the implementation.
/// Each test describes expected behavior:
///   1. `new` command creates correct file structure
///   2. ContractDeploy payload encoding matches node format
///   3. ContractCall payload encoding matches node format
///   4. Contract address derivation (blake3) matches node's derive_contract_address
///   5. Transaction signing produces valid Ed25519 signature
///   6. Edge cases: empty args, null bytes, boundary nonces

#[cfg(test)]
mod new_command {
    use crate::new::create_contract_project;
    use tempfile::TempDir;

    #[test]
    fn creates_cargo_toml_with_claw_sdk_dependency() {
        let tmp = TempDir::new().unwrap();
        create_contract_project(tmp.path(), "hello_world").unwrap();
        let cargo_toml = tmp.path().join("hello_world").join("Cargo.toml");
        assert!(cargo_toml.exists(), "Cargo.toml must be created");
        let contents = std::fs::read_to_string(&cargo_toml).unwrap();
        assert!(contents.contains("claw-sdk"), "Cargo.toml must depend on claw-sdk");
        assert!(contents.contains("hello_world"), "package name must match");
    }

    #[test]
    fn creates_src_lib_rs_with_hello_world_contract() {
        let tmp = TempDir::new().unwrap();
        create_contract_project(tmp.path(), "my_contract").unwrap();
        let lib_rs = tmp.path().join("my_contract").join("src").join("lib.rs");
        assert!(lib_rs.exists(), "src/lib.rs must be created");
        let contents = std::fs::read_to_string(&lib_rs).unwrap();
        // Must have an init function
        assert!(contents.contains("fn init"), "lib.rs must contain init function");
        // Must have get or set
        assert!(
            contents.contains("fn get") || contents.contains("fn set"),
            "lib.rs must contain get or set function"
        );
    }

    #[test]
    fn creates_readme() {
        let tmp = TempDir::new().unwrap();
        create_contract_project(tmp.path(), "my_contract").unwrap();
        let readme = tmp.path().join("my_contract").join("README.md");
        assert!(readme.exists(), "README.md must be created");
    }

    #[test]
    fn rejects_invalid_name_with_spaces() {
        let tmp = TempDir::new().unwrap();
        let result = create_contract_project(tmp.path(), "bad name");
        assert!(result.is_err(), "name with spaces must be rejected");
    }

    #[test]
    fn rejects_empty_name() {
        let tmp = TempDir::new().unwrap();
        let result = create_contract_project(tmp.path(), "");
        assert!(result.is_err(), "empty name must be rejected");
    }

    #[test]
    fn does_not_overwrite_existing_directory() {
        let tmp = TempDir::new().unwrap();
        // Create directory first
        std::fs::create_dir(tmp.path().join("existing")).unwrap();
        let result = create_contract_project(tmp.path(), "existing");
        assert!(result.is_err(), "must not overwrite existing directory");
    }

    #[test]
    fn handles_hyphen_name() {
        let tmp = TempDir::new().unwrap();
        // Hyphens are valid in Cargo package names
        create_contract_project(tmp.path(), "my-contract").unwrap();
        let dir = tmp.path().join("my-contract");
        assert!(dir.exists());
    }
}

#[cfg(test)]
mod payload_encoding {
    use crate::tx::{encode_contract_deploy_payload, encode_contract_call_payload};
    use borsh::BorshDeserialize;

    /// Mirrors the node's ContractDeployPayload struct.
    #[derive(Debug, PartialEq, borsh::BorshDeserialize)]
    struct ContractDeployPayload {
        code: Vec<u8>,
        init_method: String,
        init_args: Vec<u8>,
    }

    /// Mirrors the node's ContractCallPayload struct.
    #[derive(Debug, PartialEq, borsh::BorshDeserialize)]
    struct ContractCallPayload {
        contract: [u8; 32],
        method: String,
        args: Vec<u8>,
        value: u128,
    }

    #[test]
    fn deploy_payload_round_trips_via_borsh() {
        let code = vec![0x00, 0x61, 0x73, 0x6d]; // wasm magic
        let init_method = "init".to_string();
        let init_args = vec![0x01, 0x02, 0x03];

        let encoded = encode_contract_deploy_payload(&code, &init_method, &init_args);
        let decoded = ContractDeployPayload::deserialize(&mut encoded.as_slice()).unwrap();

        assert_eq!(decoded.code, code);
        assert_eq!(decoded.init_method, init_method);
        assert_eq!(decoded.init_args, init_args);
    }

    #[test]
    fn deploy_payload_with_empty_init_method_and_args() {
        let code = vec![1, 2, 3, 4];
        let encoded = encode_contract_deploy_payload(&code, "", &[]);
        let decoded = ContractDeployPayload::deserialize(&mut encoded.as_slice()).unwrap();
        assert_eq!(decoded.code, code);
        assert_eq!(decoded.init_method, "");
        assert_eq!(decoded.init_args, Vec::<u8>::new());
    }

    #[test]
    fn deploy_payload_with_large_wasm_code() {
        let code = vec![0xAB; 64 * 1024]; // 64KB
        let encoded = encode_contract_deploy_payload(&code, "init", &[]);
        let decoded = ContractDeployPayload::deserialize(&mut encoded.as_slice()).unwrap();
        assert_eq!(decoded.code.len(), 64 * 1024);
    }

    #[test]
    fn call_payload_round_trips_via_borsh() {
        let contract = [0xDEu8; 32];
        let method = "transfer".to_string();
        let args = vec![0xFF, 0x00, 0x12];
        let value: u128 = 1_000_000_000;

        let encoded = encode_contract_call_payload(&contract, &method, &args, value);
        let decoded = ContractCallPayload::deserialize(&mut encoded.as_slice()).unwrap();

        assert_eq!(decoded.contract, contract);
        assert_eq!(decoded.method, method);
        assert_eq!(decoded.args, args);
        assert_eq!(decoded.value, value);
    }

    #[test]
    fn call_payload_with_zero_value() {
        let contract = [0u8; 32];
        let encoded = encode_contract_call_payload(&contract, "get", &[], 0);
        let decoded = ContractCallPayload::deserialize(&mut encoded.as_slice()).unwrap();
        assert_eq!(decoded.value, 0u128);
    }

    #[test]
    fn call_payload_with_max_u128_value() {
        let contract = [1u8; 32];
        let encoded = encode_contract_call_payload(&contract, "drain", &[], u128::MAX);
        let decoded = ContractCallPayload::deserialize(&mut encoded.as_slice()).unwrap();
        assert_eq!(decoded.value, u128::MAX);
    }

    #[test]
    fn call_payload_with_unicode_method_name() {
        let contract = [0u8; 32];
        let encoded = encode_contract_call_payload(&contract, "método_特殊", &[], 0);
        let decoded = ContractCallPayload::deserialize(&mut encoded.as_slice()).unwrap();
        assert_eq!(decoded.method, "método_特殊");
    }
}

#[cfg(test)]
mod contract_address_derivation {
    use crate::tx::derive_contract_address;

    /// Test vector derived from the node's VmEngine::derive_contract_address.
    /// Formula: blake3("claw_contract_v1:" || deployer_bytes || nonce_le_bytes)
    #[test]
    fn matches_node_formula_for_zero_deployer_zero_nonce() {
        let deployer = [0u8; 32];
        let nonce: u64 = 0;

        // Manually compute the expected value
        let mut buf = Vec::new();
        buf.extend_from_slice(b"claw_contract_v1:");
        buf.extend_from_slice(&deployer);
        buf.extend_from_slice(&nonce.to_le_bytes());
        let expected = *blake3::hash(&buf).as_bytes();

        let result = derive_contract_address(&deployer, nonce);
        assert_eq!(result, expected, "must match node's derive_contract_address formula");
    }

    #[test]
    fn matches_node_formula_for_known_deployer_and_nonce() {
        let deployer = [0xABu8; 32];
        let nonce: u64 = 42;

        let mut buf = Vec::new();
        buf.extend_from_slice(b"claw_contract_v1:");
        buf.extend_from_slice(&deployer);
        buf.extend_from_slice(&nonce.to_le_bytes());
        let expected = *blake3::hash(&buf).as_bytes();

        let result = derive_contract_address(&deployer, nonce);
        assert_eq!(result, expected);
    }

    #[test]
    fn different_nonces_produce_different_addresses() {
        let deployer = [1u8; 32];
        let addr1 = derive_contract_address(&deployer, 1);
        let addr2 = derive_contract_address(&deployer, 2);
        assert_ne!(addr1, addr2, "different nonces must produce different addresses");
    }

    #[test]
    fn different_deployers_produce_different_addresses() {
        let d1 = [1u8; 32];
        let d2 = [2u8; 32];
        let addr1 = derive_contract_address(&d1, 0);
        let addr2 = derive_contract_address(&d2, 0);
        assert_ne!(addr1, addr2, "different deployers must produce different addresses");
    }

    #[test]
    fn nonce_overflow_boundary() {
        let deployer = [0u8; 32];
        // u64::MAX should not panic
        let addr = derive_contract_address(&deployer, u64::MAX);
        assert_ne!(addr, [0u8; 32], "max nonce must produce a valid address");
    }

    #[test]
    fn prefix_is_domain_separated() {
        // Ensure the claw_contract_v1: prefix is used (not empty)
        let deployer = [0u8; 32];
        let nonce: u64 = 0;

        // Without prefix
        let mut buf_no_prefix = Vec::new();
        buf_no_prefix.extend_from_slice(&deployer);
        buf_no_prefix.extend_from_slice(&nonce.to_le_bytes());
        let without_prefix = *blake3::hash(&buf_no_prefix).as_bytes();

        let result = derive_contract_address(&deployer, nonce);
        assert_ne!(result, without_prefix, "prefix must be included in hash");
    }
}

#[cfg(test)]
mod transaction_signing {
    use crate::tx::{build_and_sign_transaction, TxType};
    use ed25519_dalek::{SigningKey, VerifyingKey};

    fn test_signing_key() -> SigningKey {
        // Deterministic test key: 32 bytes of 0x42
        SigningKey::from_bytes(&[0x42u8; 32])
    }

    #[test]
    fn signed_transaction_verifies_with_public_key() {
        let signing_key = test_signing_key();
        let payload = vec![0x01, 0x02, 0x03];

        let tx = build_and_sign_transaction(TxType::ContractDeploy, &signing_key, 1, payload);

        // Verify: reconstruct signable bytes (tx_type || from || nonce_le || payload)
        let mut signable = Vec::new();
        signable.push(6u8); // ContractDeploy = 6
        signable.extend_from_slice(&tx.from);
        signable.extend_from_slice(&tx.nonce.to_le_bytes());
        signable.extend_from_slice(&tx.payload);

        let verifying_key = VerifyingKey::from(&signing_key);
        let sig = ed25519_dalek::Signature::from_bytes(&tx.signature);
        verifying_key.verify_strict(&signable, &sig).expect("signature must be valid");
    }

    #[test]
    fn from_field_matches_public_key() {
        let signing_key = test_signing_key();
        let tx = build_and_sign_transaction(TxType::ContractCall, &signing_key, 5, vec![]);
        let verifying_key = VerifyingKey::from(&signing_key);
        assert_eq!(tx.from, verifying_key.to_bytes(), "from field must equal public key bytes");
    }

    #[test]
    fn nonce_is_stored_correctly() {
        let signing_key = test_signing_key();
        let tx = build_and_sign_transaction(TxType::ContractDeploy, &signing_key, 99, vec![]);
        assert_eq!(tx.nonce, 99, "nonce must be stored as provided");
    }

    #[test]
    fn tx_type_is_stored_correctly() {
        let signing_key = test_signing_key();
        let tx_deploy = build_and_sign_transaction(TxType::ContractDeploy, &signing_key, 1, vec![]);
        let tx_call = build_and_sign_transaction(TxType::ContractCall, &signing_key, 1, vec![]);
        assert_eq!(tx_deploy.tx_type as u8, 6u8);
        assert_eq!(tx_call.tx_type as u8, 7u8);
    }

    #[test]
    fn different_nonces_produce_different_signatures() {
        let signing_key = test_signing_key();
        let payload = vec![1, 2, 3];
        let tx1 = build_and_sign_transaction(TxType::ContractDeploy, &signing_key, 1, payload.clone());
        let tx2 = build_and_sign_transaction(TxType::ContractDeploy, &signing_key, 2, payload);
        assert_ne!(tx1.signature, tx2.signature, "different nonces must produce different signatures");
    }

    #[test]
    fn empty_payload_signs_successfully() {
        let signing_key = test_signing_key();
        let tx = build_and_sign_transaction(TxType::ContractDeploy, &signing_key, 0, vec![]);
        assert_eq!(tx.payload.len(), 0);
        // Signature must be 64 bytes (non-zero)
        assert!(!tx.signature.iter().all(|&b| b == 0), "signature must not be all zeros");
    }

    #[test]
    fn large_payload_signs_successfully() {
        let signing_key = test_signing_key();
        let payload = vec![0xFFu8; 100_000]; // 100KB
        let tx = build_and_sign_transaction(TxType::ContractDeploy, &signing_key, 1, payload);
        assert!(!tx.signature.iter().all(|&b| b == 0));
    }
}

#[cfg(test)]
mod key_loading {
    use crate::key::load_signing_key;
    use tempfile::TempDir;
    use std::io::Write;

    #[test]
    fn loads_key_from_hex_string() {
        let hex = "4242424242424242424242424242424242424242424242424242424242424242";
        let key = load_signing_key(hex).unwrap();
        assert_eq!(key.to_bytes(), [0x42u8; 32]);
    }

    #[test]
    fn loads_key_from_hex_with_0x_prefix() {
        let hex = "0x4242424242424242424242424242424242424242424242424242424242424242";
        let key = load_signing_key(hex).unwrap();
        assert_eq!(key.to_bytes(), [0x42u8; 32]);
    }

    #[test]
    fn loads_key_from_file_path() {
        let tmp = TempDir::new().unwrap();
        let key_file = tmp.path().join("key.hex");
        let mut f = std::fs::File::create(&key_file).unwrap();
        writeln!(f, "4242424242424242424242424242424242424242424242424242424242424242").unwrap();

        let path_str = key_file.to_str().unwrap();
        let key = load_signing_key(path_str).unwrap();
        assert_eq!(key.to_bytes(), [0x42u8; 32]);
    }

    #[test]
    fn rejects_too_short_hex() {
        let hex = "4242"; // only 2 bytes
        let result = load_signing_key(hex);
        assert!(result.is_err(), "short hex must be rejected");
    }

    #[test]
    fn rejects_invalid_hex_characters() {
        let hex = "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ";
        let result = load_signing_key(hex);
        assert!(result.is_err(), "invalid hex must be rejected");
    }

    #[test]
    fn rejects_nonexistent_file_path() {
        let result = load_signing_key("/nonexistent/path/to/key.hex");
        assert!(result.is_err(), "nonexistent file must be rejected");
    }
}

#[cfg(test)]
mod args_parsing {
    use crate::args::parse_hex_or_empty;

    #[test]
    fn parses_hex_with_0x_prefix() {
        let result = parse_hex_or_empty("0x0102ff").unwrap();
        assert_eq!(result, vec![0x01, 0x02, 0xFF]);
    }

    #[test]
    fn parses_hex_without_prefix() {
        let result = parse_hex_or_empty("0102ff").unwrap();
        assert_eq!(result, vec![0x01, 0x02, 0xFF]);
    }

    #[test]
    fn parses_0x_alone_as_empty() {
        let result = parse_hex_or_empty("0x").unwrap();
        assert_eq!(result, Vec::<u8>::new());
    }

    #[test]
    fn parses_empty_string_as_empty() {
        let result = parse_hex_or_empty("").unwrap();
        assert_eq!(result, Vec::<u8>::new());
    }

    #[test]
    fn rejects_odd_length_hex() {
        let result = parse_hex_or_empty("0x01f");
        assert!(result.is_err(), "odd-length hex must be rejected");
    }

    #[test]
    fn rejects_non_hex_characters() {
        let result = parse_hex_or_empty("0xGGGG");
        assert!(result.is_err(), "non-hex characters must be rejected");
    }
}
