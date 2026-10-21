import CryptoKit
import Foundation
import Security

// The signing seed stays in the login Keychain. Only the public key or a
// detached signature is written to stdout; private material is never printed.
let service = "com.fourquadrants.gpui.sparkle"
let account = "ed25519"

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func signingKey(create: Bool, privateKeyStdin: Bool = false) throws -> Curve25519.Signing.PrivateKey {
    if privateKeyStdin {
        let encoded = String(data: FileHandle.standardInput.readDataToEndOfFile(), encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard let encoded, let seed = Data(base64Encoded: encoded), seed.count == 32 else {
            fail("Standard input must contain a base64-encoded 32-byte Ed25519 seed.")
        }
        return try Curve25519.Signing.PrivateKey(rawRepresentation: seed)
    }
    let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service,
        kSecAttrAccount as String: account,
        kSecReturnData as String: true,
        kSecMatchLimit as String: kSecMatchLimitOne,
    ]
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecSuccess, let seed = result as? Data {
        return try Curve25519.Signing.PrivateKey(rawRepresentation: seed)
    }
    guard status == errSecItemNotFound, create else {
        fail("Cannot read the update signing key from Keychain (OSStatus \(status)).")
    }
    let key = Curve25519.Signing.PrivateKey()
    let attributes: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service,
        kSecAttrAccount as String: account,
        kSecAttrLabel as String: "FourQuadrants GPUI Update Signing Key",
        kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        kSecValueData as String: key.rawRepresentation,
    ]
    let added = SecItemAdd(attributes as CFDictionary, nil)
    guard added == errSecSuccess else {
        fail("Cannot save the update signing key in Keychain (OSStatus \(added)).")
    }
    return key
}

let arguments = CommandLine.arguments

do {
    switch arguments.dropFirst().first {
    case "self-test" where arguments.count == 2:
        // RFC 8032 section 7.1, public test vector 1. No Keychain access.
        func bytes(_ hex: String) -> Data {
            let characters = Array(hex)
            return Data(stride(from: 0, to: characters.count, by: 2).map {
                UInt8(String(characters[$0...($0 + 1)]), radix: 16)!
            })
        }
        let key = try Curve25519.Signing.PrivateKey(rawRepresentation: bytes("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"))
        let publicKey = bytes("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
        let expected = bytes("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")
        let signature = try key.signature(for: Data())
        // CryptoKit may randomize signatures; verify both its result and the
        // deterministic published vector against the same public key.
        guard key.publicKey.rawRepresentation == publicKey,
              key.publicKey.isValidSignature(expected, for: Data()),
              key.publicKey.isValidSignature(signature, for: Data()),
              !key.publicKey.isValidSignature(signature, for: Data([1])) else {
            fail("Ed25519 RFC 8032 public-key/signature verification failed.")
        }
        print("Ed25519 RFC 8032 test vector and tamper rejection passed.")
    case "public" where arguments.count == 2:
        print(try signingKey(create: true).publicKey.rawRepresentation.base64EncodedString())
    case "sign" where arguments.count == 3 || (arguments.count == 4 && arguments[3] == "--private-key-stdin"):
        let archive = try Data(contentsOf: URL(fileURLWithPath: arguments[2]), options: .mappedIfSafe)
        print(try signingKey(create: false, privateKeyStdin: arguments.count == 4).signature(for: archive).base64EncodedString())
    case "verify" where arguments.count == 5:
        guard let signature = Data(base64Encoded: arguments[3]),
              let publicBytes = Data(base64Encoded: arguments[4]) else {
            fail("Invalid base64 signature or public key.")
        }
        let key = try Curve25519.Signing.PublicKey(rawRepresentation: publicBytes)
        let archive = try Data(contentsOf: URL(fileURLWithPath: arguments[2]), options: .mappedIfSafe)
        guard key.isValidSignature(signature, for: archive) else {
            fail("Update signature does not match the bundled public key.")
        }
        print("Update signature verified.")
    default:
        fail("Usage: swift scripts/update-key.swift self-test | public | sign ARCHIVE [--private-key-stdin] | verify ARCHIVE SIGNATURE PUBLIC_KEY")
    }
} catch {
    fail("Update signing failed: \(error.localizedDescription)")
}
