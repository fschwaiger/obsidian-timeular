import {
	App,
	Plugin,
	PluginSettingTab,
	Setting,
	addIcon,
	Notice,
} from "obsidian";
import { ChildProcess, spawn } from "child_process";
import * as path from "path";
import { existsSync } from "fs";

type Side = "BF" | "BR" | "BL" | "BB" | "TF" | "TR" | "TL" | "TB" | "__";
type State = "disconnected" | "connecting" | "connected" | "unavailable";

type Action = {
	command?: string;
	template?: string;
	actionSet?: string;
};

type ActionSet = {
	[key in Side]: Action;
};

function decodeSide(
	data: number
): Side {
	switch (data) {
		case 0x01:
			return "BF";
		case 0x02:
			return "BR";
		case 0x03:
			return "BL";
		case 0x04:
			return "BB";
		case 0x05:
			return "TF";
		case 0x06:
			return "TR";
		case 0x07:
			return "TL";
		case 0x08:
			return "TB";
		default:
			return "__";
	}
}

interface BluetoothTimeTrackerPluginSettings {
	deviceName: string;
	templateTargetFile: string;
	activeActionSet: string;
	actionSetsByName: { [key: string]: ActionSet };
	companionPort: number;
	companionHost: string;
	deviceMacAddress: string;
}

const DEFAULT_SETTINGS: BluetoothTimeTrackerPluginSettings = {
	deviceName: "Timeular Tracker",
	deviceMacAddress: "D1:16:15:2C:DE:8B",
	templateTargetFile: "Daily/{{date}}.md",
	activeActionSet: "default",
	companionPort: 9999,
	companionHost: "127.0.0.1",
	actionSetsByName: {
		default: ["BF", "BR", "BL", "BB", "TF", "TR", "TL", "TB", "__"].reduce(
			(set: ActionSet, side: Side) => {
				set[side] = {
					template: `{{time}} ${side}`,
				};
				return set;
			},
			{} as ActionSet
		),
	},
};

export default class BluetoothTimeTrackerPlugin extends Plugin {
	settings: BluetoothTimeTrackerPluginSettings;
	state: State = "disconnected";
	side: Side;
	battery: number = 0;
	ws: WebSocket | undefined = undefined;
	statusBarItemEl: HTMLElement;
	ribbonIconEl: HTMLElement;
	reconnectTimeout: NodeJS.Timeout | undefined;
	companionProcess: ChildProcess | undefined = undefined;
	companionStarting: boolean = false;

	async onReconnect() {
		if (this.state === "disconnected" || this.state == "unavailable") {
			return this.onConnect();
		} else if (this.state === "connecting") {
			return this.onCancel();
		} else {
			return this.onDisconnect();
		}
	}

	async onConnect() {
		this.updateState("connecting");

		// Try to start companion if not running
		if (!this.companionProcess && !this.companionStarting) {
			await this.startCompanionIfNeeded();
		}

		try {
			const url = `ws://${this.settings.companionHost}:${this.settings.companionPort}`;
			this.ws = new WebSocket(url);

			this.ws.onopen = () => {
				this.updateState("connected");
			};

			this.ws.onmessage = (event) => {
				try {
					const data = JSON.parse(event.data);

					if (data.error) {
						this.updateState("disconnected");
						return;
					}

					if (data.indicator !== undefined) {
						this.side = decodeSide(data.indicator);
						this.battery = data.battery || 0;
						this.updateState("connected");
					}
				} catch (error) {
					console.error('Error parsing message:', error);
				}
			};

			this.ws.onerror = (error) => {
				console.error('WebSocket error:', error);
				this.updateState("unavailable");
			};

			this.ws.onclose = () => {
				this.ws = undefined;
				this.updateState("disconnected");

				if (this.state === "connected" || this.state === "connecting") {
					this.reconnectTimeout = setTimeout(() => {
						this.onConnect();
					}, 5000);
				}
			};

		} catch (error) {
			this.updateState("unavailable");
			console.error('Error connecting to companion app:', error);
		}
	}

	async onCancel() {
		// Clear any reconnect timeout
		if (this.reconnectTimeout) {
			clearTimeout(this.reconnectTimeout);
			this.reconnectTimeout = undefined;
		}

		// Close WebSocket if connected
		if (this.ws) {
			this.ws.close();
			this.ws = undefined;
		}

		this.updateState("disconnected");
	}

	async onDisconnect() {
		// Clear any reconnect timeout
		if (this.reconnectTimeout) {
			clearTimeout(this.reconnectTimeout);
			this.reconnectTimeout = undefined;
		}

		// Close WebSocket
		if (this.ws) {
			this.ws.close();
			this.ws = undefined;
		}

		this.updateState("disconnected");
	}

	async startCompanionIfNeeded() {
		if (this.companionStarting || this.companionProcess) {
			return;
		}

		this.companionStarting = true;

		// Get the plugin directory - use the adapter to get the proper file system path
		const pluginDir = (this.app.vault.adapter as any).getBasePath();
		const companionDir = path.join(pluginDir, ".obsidian", "plugins", "obsidian-ble-tracker", "companion");
		const binaryPath = path.join(companionDir, "target", "release", "ble_tracker_companion.exe");
		const debugBinaryPath = path.join(companionDir, "target", "debug", "ble_tracker_companion.exe");

		try {
			let execPath: string;
			if (existsSync(binaryPath)) {
				execPath = binaryPath;
			} else if (existsSync(debugBinaryPath)) {
				execPath = debugBinaryPath;
			} else {
				new Notice("Companion app not built. Please run 'cd companion && cargo build'");
				this.updateState("unavailable");
				this.companionStarting = false;
				return;
			}

			this.companionProcess = spawn(execPath, [], {
				env: {
					...process.env,
					TRACKER_TCP_PORT: String(this.settings.companionPort),
					TRACKER_MAC_ADDRESS: this.settings.deviceMacAddress,
					TRACKER_BATTERY_UUID: "00002a19-0000-1000-8000-00805f9b34fb",
					TRACKER_INDICATOR_UUID: "c7e70012-c847-11e6-8175-8c89a55d403c",
				},
				cwd: companionDir,
				stdio: ['ignore', 'pipe', 'pipe'],
			});

			this.companionProcess.stderr?.on('data', (data) => {
				console.error(`Companion: ${data}`);
			});

			this.companionProcess.on('exit', (code) => {
				this.companionProcess = undefined;

				if (code !== 0 && code !== null) {
					new Notice(`Companion failed (exit code: ${code})`);
				}

				if (this.state === "connected" || this.state === "connecting") {
					this.updateState("disconnected");
				}
			});

			await new Promise(resolve => setTimeout(resolve, 2000));

		} catch (error) {
			console.error("Failed to start companion:", error);
			new Notice("Failed to start companion app. Check console for details.");
			this.updateState("unavailable");
		} finally {
			this.companionStarting = false;
		}
	}

	stopCompanion() {
		if (this.companionProcess) {
			this.companionProcess.kill();
			this.companionProcess = undefined;
		}
	}

	async onload() {
		await this.loadSettings();

		addIcon(
			"time-tracker",
			'<g transform="scale(4,4)"><path stroke="currentColor" fill="none" d="M2.7 10.3a2.41 2.41 0 0 0 0 3.41l7.59 7.59a2.41 2.41 0 0 0 3.41 0l7.59-7.59a2.41 2.41 0 0 0 0-3.41L13.7 2.71a2.41 2.41 0 0 0-3.41 0z"/><path stroke="currentColor" d="M8 12h8"/></g>'
		);

		this.ribbonIconEl = this.addRibbonIcon(
			"time-tracker",
			"Time Tracker",
			this.onReconnect.bind(this)
		);

		this.statusBarItemEl = this.addStatusBarItem();
		this.statusBarItemEl.addClass("mod-clickable");
		this.statusBarItemEl.addEventListener("click", this.onReconnect.bind(this));

		this.addSettingTab(new BluetoothTimeTrackerSettingTab(this.app, this));

		this.updateState("disconnected");
	}

	onunload() {
		// Clean up on plugin unload
		if (this.reconnectTimeout) {
			clearTimeout(this.reconnectTimeout);
		}
		if (this.ws) {
			this.ws.close();
		}
		// Stop the companion app
		this.stopCompanion();
	}

	async loadSettings() {
		this.settings = Object.assign(
			{},
			DEFAULT_SETTINGS,
			await this.loadData()
		);
	}

	async saveSettings() {
		await this.saveData(this.settings);
	}

	async updateState(state: State | undefined = undefined) {
		if (state !== undefined) {
			this.state = state;
		}

		this.ribbonIconEl.setAttr("aria-label", `Time Tracker: ${this.state}`);

		if (this.state == "connected") {
			this.ribbonIconEl.style.opacity = "1.0";
			this.ribbonIconEl.style.color = "rgb(50, 200, 0)";
			const batteryInfo = this.battery ? ` ${this.battery}%` : '';
			this.statusBarItemEl.setText(`◇${batteryInfo} <${this.side || 'N/A'}>`);
		} else if (this.state == "connecting") {
			this.ribbonIconEl.style.opacity = "1.0";
			this.ribbonIconEl.style.color = "rgb(80, 160, 255)";
			this.statusBarItemEl.setText("◇ scanning ...");
		} else if (this.state == "unavailable") {
			this.ribbonIconEl.style.opacity = "0.5";
			this.ribbonIconEl.style.color = "inherit";
			this.statusBarItemEl.setText("◇ unavailable");
		} else {
			this.ribbonIconEl.style.opacity = "1.0";
			this.ribbonIconEl.style.color = "inherit";
			this.statusBarItemEl.setText("◇ disconnected");
		}
	}
}

class BluetoothTimeTrackerSettingTab extends PluginSettingTab {
	plugin: BluetoothTimeTrackerPlugin;

	constructor(app: App, plugin: BluetoothTimeTrackerPlugin) {
		super(app, plugin);
		this.plugin = plugin;
	}

	display(): void {
		const { containerEl } = this;

		containerEl.empty();

		containerEl.createEl("h2", { text: "Bluetooth Time Tracker Settings" });

		containerEl.createEl("p", {
			text: "This plugin requires the companion app to be running. The companion app handles Bluetooth communication with your device.",
			cls: "setting-item-description"
		});

		new Setting(containerEl)
			.setName("Companion Host")
			.setDesc("The host address of the companion app")
			.addText((text) =>
				text
					.setPlaceholder("127.0.0.1")
					.setValue(this.plugin.settings.companionHost)
					.onChange(async (value) => {
						this.plugin.settings.companionHost = value;
						await this.plugin.saveSettings();
					})
			);

		new Setting(containerEl)
			.setName("Companion Port")
			.setDesc("The TCP port of the companion app")
			.addText((text) =>
				text
					.setPlaceholder("9999")
					.setValue(String(this.plugin.settings.companionPort))
					.onChange(async (value) => {
						this.plugin.settings.companionPort = parseInt(value) || 9999;
						await this.plugin.saveSettings();
					})
			);

		new Setting(containerEl)
			.setName("Device Name")
			.setDesc("The name of the time tracker device (for display only)")
			.addText((text) =>
				text
					.setPlaceholder("Timeular Tracker")
					.setValue(this.plugin.settings.deviceName)
					.onChange(async (value) => {
						this.plugin.settings.deviceName = value;
						await this.plugin.saveSettings();
					})
			);

		new Setting(containerEl)
			.setName("Device MAC Address")
			.setDesc("The Bluetooth MAC address of your device (e.g., D1:16:15:2C:DE:8B)")
			.addText((text) =>
				text
					.setPlaceholder("D1:16:15:2C:DE:8B")
					.setValue(this.plugin.settings.deviceMacAddress)
					.onChange(async (value) => {
						this.plugin.settings.deviceMacAddress = value;
						await this.plugin.saveSettings();
					})
			);

		new Setting(containerEl)
			.setName("Template Target File")
			.setDesc("The file to write the time entries to")
			.addText((text) =>
				text
					.setPlaceholder("Enter the template target file")
					.setValue(this.plugin.settings.templateTargetFile)
					.onChange(async (value) => {
						this.plugin.settings.templateTargetFile = value;
						await this.plugin.saveSettings();
					})
			);
	}
}
