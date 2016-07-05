// This file is released under the same terms as Rust itself.

use openssl::crypto::hash::Type;
use openssl::crypto::hmac::hmac;
use openssl::crypto::memcmp::eq as secure_eq;

pub const SHA1_LEN: usize = 40;

pub fn generate_sha1_hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
	hmac(Type::SHA1, key, data)
}

pub fn verify_sha1_hmac(key: &[u8], data: &[u8], signature: &[u8]) -> bool {
	let expected_signature = hmac(Type::SHA1, key, data);
	secure_eq(&expected_signature, signature)
}