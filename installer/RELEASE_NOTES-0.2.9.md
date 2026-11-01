# MTT File Manager v0.2.9

## Choose your installer

This release offers two Windows x64 installers:

| Installer | Built-in updates |
| --- | --- |
| **`MTT-File-Manager-Setup-0.2.9.exe`** | Standard edition: online version checks, update notifications, and in-app download and installation. Automatic installation is optional and off by default. |
| **`MTT-File-Manager-Setup-0.2.9-no-update.exe`** | No-update edition: the entire built-in update component is removed, including online version checks, update notifications, update controls, and the update helper. Download and install future versions manually. |

Both editions provide the same file-manager features apart from built-in updating. The no-update edition is not a network-disabled edition: features such as network-share browsing remain available.

### Why offer two editions?

During release testing, installers containing update components received antivirus alerts. We believe these are false positives and have requested reviews from Google and Ikarus; responses are pending. The no-update installer received no detections in the VirusTotal scan reported during testing.

Detection results can change. A clean scan is not a security guarantee, and an alert alone does not confirm malware. We are offering both editions so you can choose whether to include built-in updating. We do not recommend disabling antivirus protection or adding exclusions to install either edition.

The earlier standard-installer report discussed during testing concerns a different SHA-256 from the standard installer supplied here. Do not treat that report as the scan result for this exact file. Use the hashes below to identify the supplied installers.

### Installation and future updates

- Install **one edition**, not both. These are alternative builds of the same application and use the same installation location; they are not side-by-side installations.
- Close MTT before manually installing either edition. You can switch editions by running the corresponding installer; no uninstall is required.
- With the no-update edition, visit the GitHub Releases page yourself to check for new versions. Choose the `-no-update.exe` installer again if you want to keep built-in updating excluded.
- The standard edition's built-in updater selects the standard installer, not the no-update installer.

### SHA-256

```text
46482a7fd64275f9e2669e69c1ac0cdca41cb131a98b4efe16aabb6d30798021  MTT-File-Manager-Setup-0.2.9.exe
8eb5e8106ba9819d12b51b637f56634828cb61cfa9c1d3cd045726fad6b92202  MTT-File-Manager-Setup-0.2.9-no-update.exe
```
