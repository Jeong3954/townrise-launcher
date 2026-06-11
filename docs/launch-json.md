# TownRise launcher `launch.json`

The launcher starts Minecraft by reading `launch.json` from the updated instance directory:

```text
<TownRiseLauncher data dir>/instance/launch.json
```

The same update manifest that installs `mods/townrise.jar` can also ship this file as `launch.json`.

Example shape:

```json
{
  "javaExecutable": "java",
  "workingDirectory": ".",
  "jvmArgs": ["-Xmx4G", "-Dfile.encoding=UTF-8"],
  "classpath": [
    "libraries/example-library.jar",
    "versions/townrise/neoforge-client.jar"
  ],
  "mainClass": "net.minecraft.client.main.Main",
  "gameArgs": [
    "--username",
    "Player",
    "--version",
    "TownRise-1.21.1",
    "--gameDir",
    ".",
    "--assetsDir",
    "assets",
    "--assetIndex",
    "17",
    "--accessToken",
    "0",
    "--uuid",
    "00000000-0000-0000-0000-000000000000"
  ]
}
```

Security constraints:

- `classpath` and `workingDirectory` must be instance-relative paths.
- `..`, absolute paths, Windows drive paths, and NUL bytes are rejected.
- The launcher uses `std::process::Command`, not shell execution.

Production note:

A real online-mode Minecraft login flow still needs Microsoft/Minecraft authentication tokens. Until that is wired, this is suitable for a prepared/offline test instance or a generated launch config that already contains valid launch arguments.
