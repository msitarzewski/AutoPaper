using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using AutoPaper.Core;
using Windows.Win32;
using Windows.Win32.Security.Credentials;

namespace AutoPaper.Services;

/// <summary>
/// API keys in Windows Credential Manager: generic credentials named "AutoPaper:&lt;account&gt;" (the accounts come
/// from the core: "openai.api_key", "google.api_key", "openai_compatible.api_key@&lt;origin&gt;"), stored for this
/// user on this PC only (CRED_PERSIST_LOCAL_MACHINE: protected by Windows for the signed-in user, never roamed).
/// PasswordVault isn't used: it roams with the Microsoft account and holds 20 entries (docs/research/windows.md).
/// The core calls this from its own threads; Credential Manager is safe to call from any thread.
/// </summary>
internal sealed unsafe class CredentialStore(string prefix = CredentialStore.AppPrefix) : SecretStore
{
    public const string AppPrefix = "AutoPaper:";
    /// <summary>CRED_MAX_CREDENTIAL_BLOB_SIZE.</summary>
    private const int MaxBlobBytes = 5 * 512;

    /// <summary>The Credential Manager target for an account: "AutoPaper:openai.api_key". (Tests use their own
    /// prefix, so they never touch the person's keys.)</summary>
    public string TargetName(string account) => prefix + account;

    public string? Get(string account)
    {
        CREDENTIALW* credential;
        fixed (char* target = TargetName(account))
        {
            if (!PInvoke.CredRead(target, CRED_TYPE.CRED_TYPE_GENERIC, 0, &credential))
            {
                return null; // ERROR_NOT_FOUND, or nothing readable: the core reads it as "no key".
            }
        }
        try
        {
            if (credential->CredentialBlob == null || credential->CredentialBlobSize == 0)
            {
                return null;
            }
            var value = Encoding.Unicode.GetString(credential->CredentialBlob, (int)credential->CredentialBlobSize);
            return value.Length == 0 ? null : value;
        }
        finally
        {
            PInvoke.CredFree(credential);
        }
    }

    /// <summary>Saves the key; an empty value deletes it. Throws <see cref="Win32Exception"/> when Windows refuses.</summary>
    public void Set(string account, string value)
    {
        value = value.Trim();
        if (value.Length == 0)
        {
            Delete(account);
            return;
        }
        var blob = Encoding.Unicode.GetBytes(value);
        try
        {
            if (blob.Length > MaxBlobBytes)
            {
                throw new ArgumentException("That key is too long for Credential Manager.", nameof(value));
            }
            fixed (char* target = TargetName(account))
            fixed (char* user = account)
            fixed (byte* data = blob)
            {
                var credential = new CREDENTIALW
                {
                    Type = CRED_TYPE.CRED_TYPE_GENERIC,
                    TargetName = target,
                    UserName = user,
                    CredentialBlobSize = (uint)blob.Length,
                    CredentialBlob = data,
                    Persist = CRED_PERSIST.CRED_PERSIST_LOCAL_MACHINE,
                };
                if (!PInvoke.CredWrite(&credential, 0))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
            }
        }
        finally
        {
            CryptographicOperations.ZeroMemory(blob);
        }
    }

    public void Delete(string account)
    {
        fixed (char* target = TargetName(account))
        {
            // A missing credential is already deleted.
            PInvoke.CredDelete(target, CRED_TYPE.CRED_TYPE_GENERIC, 0);
        }
    }

    public bool Has(string account) => !string.IsNullOrEmpty(Get(account));
}
