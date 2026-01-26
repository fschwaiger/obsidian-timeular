# Obsidian BLE Time Tracker Adapter Plugin

This plugin for [Obsidian](https://obsidian.md) connects via Bluetooth Low Energy (BLE) to your time tracker device, such as the [Timeular tracker](https://timeular.com/tracker/) to trigger actions in your vault when the tracker is moved:

1. It can insert a timestamp and custom text into your daily note.
2. It can trigger vault actions (e.g. from other plugins) on state change.

## Architecture

This plugin uses a **companion app** architecture with automatic process management:

- **Companion App** (Rust): A native application that handles the Bluetooth connection to your device. It sends device state updates over TCP.
- **Obsidian Plugin** (TypeScript): Automatically starts the companion app if needed, connects via WebSocket to receive device updates, and triggers vault actions.

## Setup Instructions

### 1. Build the Companion App

Build the companion app once:

```bash
cd companion
cargo build --release  # or just 'cargo build' for debug build
```

The plugin will automatically use the release build if available, otherwise it will use the debug build.

### 2. Install the Obsidian Plugin

Copy the plugin files to your vault:

```bash
cp main.js manifest.json /path/to/your/vault/.obsidian/plugins/obsidian-ble-time-tracker/
```

Or build from source:

```bash
npm install
npm run build
```

### 3. Use the Plugin

1. Enable the plugin in Obsidian settings
2. Click the ribbon icon or status bar item to connect

**That's it!** The plugin will automatically:
- Start the companion app if it's not running
- Connect to your Bluetooth device
- Display device status in the status bar
- Clean up the companion process when the plugin is unloaded

## Configuration

Open plugin settings to configure:
- **Companion Host**: Default `127.0.0.1` (localhost)
- **Companion Port**: Default `9999`
- **Device Name**: Your tracker device name (e.g., "Timeular Tracker")
- **Template Target File**: Where to write time entries

## Development

------------------

## Releasing new releases

- Update your `manifest.json` with your new version number, such as `1.0.1`, and the minimum Obsidian version required for your latest release.
- Update your `versions.json` file with `"new-plugin-version": "minimum-obsidian-version"` so older versions of Obsidian can download an older version of your plugin that's compatible.
- Create new GitHub release using your new version number as the "Tag version". Use the exact version number, don't include a prefix `v`. See here for an example: https://github.com/obsidianmd/obsidian-sample-plugin/releases
- Upload the files `manifest.json`, `main.js`, `styles.css` as binary attachments. Note: The manifest.json file must be in two places, first the root path of your repository and also in the release.
- Publish the release.

> You can simplify the version bump process by running `npm version patch`, `npm version minor` or `npm version major` after updating `minAppVersion` manually in `manifest.json`.
> The command will bump version in `manifest.json` and `package.json`, and add the entry for the new version to `versions.json`

## Adding your plugin to the community plugin list

- Check the [plugin guidelines](https://docs.obsidian.md/Plugins/Releasing/Plugin+guidelines).
- Publish an initial version.
- Make sure you have a `README.md` file in the root of your repo.
- Make a pull request at https://github.com/obsidianmd/obsidian-releases to add your plugin.

## How to use

- Clone this repo.
- Make sure your NodeJS is at least v16 (`node --version`).
- `npm i` or `yarn` to install dependencies.
- `npm run dev` to start compilation in watch mode.

## Manually installing the plugin

- Copy over `main.js`, `styles.css`, `manifest.json` to your vault `VaultFolder/.obsidian/plugins/your-plugin-id/`.

## Improve code quality with eslint (optional)
- [ESLint](https://eslint.org/) is a tool that analyzes your code to quickly find problems. You can run ESLint against your plugin to find common bugs and ways to improve your code. 
- To use eslint with this project, make sure to install eslint from terminal:
  - `npm install -g eslint`
- To use eslint to analyze this project use this command:
  - `eslint main.ts`
  - eslint will then create a report with suggestions for code improvement by file and line number.
- If your source code is in a folder, such as `src`, you can use eslint with this command to analyze all files in that folder:
  - `eslint .\src\`

## Funding URL

You can include funding URLs where people who use your plugin can financially support it.

The simple way is to set the `fundingUrl` field to your link in your `manifest.json` file:

```json
{
    "fundingUrl": "https://buymeacoffee.com"
}
```

If you have multiple URLs, you can also do:

```json
{
    "fundingUrl": {
        "Buy Me a Coffee": "https://buymeacoffee.com",
        "GitHub Sponsor": "https://github.com/sponsors",
        "Patreon": "https://www.patreon.com/"
    }
}
```

## API Documentation

See https://github.com/obsidianmd/obsidian-api
