using System.Net.Http;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace NostrVpn.Windows.Services;

public sealed class UpdateService
{
    private static readonly Uri HtreeManifestUri = new("https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest/release.json");
    private static readonly Uri GithubLatestReleaseUri = new("https://api.github.com/repos/mmalmi/nostr-vpn/releases/latest");
    private static readonly HttpClient Http = new();
    private static readonly JsonSerializerOptions JsonOptions = new() { PropertyNameCaseInsensitive = true };

    public static bool SkipOpen => Environment.GetEnvironmentVariable("NVPN_UPDATE_SKIP_OPEN") == "1";

    public async Task<UpdateResult> CheckAsync(string currentVersion)
    {
        Exception? lastError = null;
        foreach (var manifestUri in ManifestUris())
        {
            try
            {
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
            catch (Exception error)
            {
                lastError = error;
            }
        }

        throw lastError ?? new InvalidOperationException("no update manifest URL configured");
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

    private static IReadOnlyList<Uri> ManifestUris()
    {
        var overrideUrl = Environment.GetEnvironmentVariable("NVPN_UPDATE_MANIFEST_URL");
        return string.IsNullOrWhiteSpace(overrideUrl)
            ? new[] { HtreeManifestUri, GithubLatestReleaseUri }
            : new[] { new Uri(overrideUrl) };
    }

    private static async Task<string> ReadStringAsync(Uri uri)
    {
        if (uri.IsFile)
        {
            return await File.ReadAllTextAsync(uri.LocalPath);
        }

        using var request = new HttpRequestMessage(HttpMethod.Get, uri);
        if (uri.Host.Equals("api.github.com", StringComparison.OrdinalIgnoreCase))
        {
            request.Headers.Accept.ParseAdd("application/vnd.github+json");
            request.Headers.UserAgent.ParseAdd("nvpn-updater");
        }
        using var response = await Http.SendAsync(request);
        response.EnsureSuccessStatusCode();
        return await response.Content.ReadAsStringAsync();
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
    [JsonPropertyName("tag_name")]
    public string TagName { get => Tag; set { if (!string.IsNullOrWhiteSpace(value)) Tag = value; } }
    public List<ReleaseAsset> Assets { get; set; } = [];
}

public sealed class ReleaseAsset
{
    public string Name { get; set; } = "";
    public string Path { get; set; } = "";
    [JsonPropertyName("browser_download_url")]
    public string BrowserDownloadUrl { get => Path; set { if (!string.IsNullOrWhiteSpace(value)) Path = value; } }
    public string Url => Path;
}
