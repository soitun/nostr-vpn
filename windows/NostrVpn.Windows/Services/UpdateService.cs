using System.Net.Http;
using System.Text.Json;

namespace NostrVpn.Windows.Services;

public sealed class UpdateService
{
    private static readonly Uri DefaultManifestUri = new("https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest/release.json");
    private static readonly HttpClient Http = new();
    private static readonly JsonSerializerOptions JsonOptions = new() { PropertyNameCaseInsensitive = true };

    public static bool SkipOpen => Environment.GetEnvironmentVariable("NVPN_UPDATE_SKIP_OPEN") == "1";

    public async Task<UpdateResult> CheckAsync(string currentVersion)
    {
        var manifestUri = ManifestUri();
        var json = await ReadStringAsync(manifestUri);
        var manifest = JsonSerializer.Deserialize<ReleaseManifest>(json, JsonOptions)
            ?? throw new InvalidOperationException("release manifest was empty");
        var asset = PreferredWindowsAsset(manifest.Assets);
        var available = VersionIsNewer(manifest.Tag, currentVersion);
        return new UpdateResult(
            available,
            manifest.Tag,
            asset?.Url is null ? null : new Uri(manifestUri, asset.Url),
            asset?.Name,
            available
                ? asset is null ? $"Update {manifest.Tag} found without a Windows asset" : $"Update {manifest.Tag} available"
                : "Up to date");
    }

    public async Task<string> DownloadAsync(Uri assetUri)
    {
        var downloadDir = Environment.GetEnvironmentVariable("NVPN_UPDATE_DOWNLOAD_DIR");
        if (string.IsNullOrWhiteSpace(downloadDir))
        {
            downloadDir = Path.Combine(Path.GetTempPath(), "NostrVpnDownloads");
        }
        Directory.CreateDirectory(downloadDir);
        var fileName = Path.GetFileName(assetUri.LocalPath);
        if (string.IsNullOrWhiteSpace(fileName))
        {
            fileName = "nostr-vpn-update.exe";
        }
        var destination = Path.Combine(downloadDir, fileName);
        if (File.Exists(destination))
        {
            File.Delete(destination);
        }
        if (assetUri.IsFile)
        {
            File.Copy(assetUri.LocalPath, destination);
        }
        else
        {
            await using var stream = await Http.GetStreamAsync(assetUri);
            await using var file = File.Create(destination);
            await stream.CopyToAsync(file);
        }
        return destination;
    }

    private static Uri ManifestUri()
    {
        var overrideUrl = Environment.GetEnvironmentVariable("NVPN_UPDATE_MANIFEST_URL");
        return string.IsNullOrWhiteSpace(overrideUrl) ? DefaultManifestUri : new Uri(overrideUrl);
    }

    private static Task<string> ReadStringAsync(Uri uri)
    {
        return uri.IsFile ? File.ReadAllTextAsync(uri.LocalPath) : Http.GetStringAsync(uri);
    }

    private static ReleaseAsset? PreferredWindowsAsset(IEnumerable<ReleaseAsset> assets)
    {
        var arch = Environment.GetEnvironmentVariable("PROCESSOR_ARCHITECTURE") ?? "";
        var preferred = arch.Contains("ARM64", StringComparison.OrdinalIgnoreCase)
            ? "windows-arm64-setup.exe"
            : "windows-x64-setup.exe";
        return assets.FirstOrDefault(asset => asset.Name.EndsWith(preferred, StringComparison.OrdinalIgnoreCase))
            ?? assets.FirstOrDefault(asset => asset.Name.EndsWith("windows-x64-setup.exe", StringComparison.OrdinalIgnoreCase));
    }

    private static bool VersionIsNewer(string candidate, string current)
    {
        var normalizedCandidate = candidate.Trim().TrimStart('v', 'V');
        var normalizedCurrent = current.Trim().TrimStart('v', 'V');
        return Version.TryParse(normalizedCandidate, out var candidateVersion)
            && Version.TryParse(normalizedCurrent, out var currentVersion)
            && candidateVersion > currentVersion;
    }
}

public sealed record UpdateResult(bool Available, string Tag, Uri? AssetUrl, string? AssetName, string Message);

public sealed class ReleaseManifest
{
    public string Tag { get; set; } = "";
    public List<ReleaseAsset> Assets { get; set; } = [];
}

public sealed class ReleaseAsset
{
    public string Name { get; set; } = "";
    public string Path { get; set; } = "";
    public string Url => Path;
}
