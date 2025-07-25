<script>
  import { run } from "svelte/legacy";

  import { invoke } from "@tauri-apps/api/core";
  import { createEventDispatcher } from "svelte";
  import Modal from "$lib/components/Modal.svelte";
  import { faPlay } from "@fortawesome/free-solid-svg-icons";
  import { onMount } from "svelte";
  import Select from "$lib/components/Select.svelte";
  import Datalist from "$lib/components/Datalist.svelte";
  import Input from "$lib/components/Input.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Fa from "svelte-fa";

  const dispatch = createEventDispatcher();

  let enabled = $state(false);
  let isSteamRunning = $state(false);
  let isRunning = $state(false);
  let settings = $state({});
  let hlaeSettings = $state({});
  let defaultHlaeSettings = $state({});
  let outputSettings = $state({});

  let startingDemo = $state("");
  let install = $state("");
  let tabIndex = $state(0);

  let demos = $state([]);

  let toggle = () => (enabled = !enabled);

  async function loadSettings() {
    settings = await invoke("load_settings");
    hlaeSettings = settings.hlae;
    defaultHlaeSettings = settings.hlae;
    outputSettings = settings.output;
    install = settings.tf_folder;
  }

  async function loadDemos() {
    try {
      let resp = await invoke("load_demos");

      demos = resp.demos.map((demo) => demo.name);
    } catch (error) {
      alert(error);
    }
  }

  async function checkSteam() {
    isSteamRunning = await invoke("is_steam_running");

    while (!isSteamRunning) {
      isSteamRunning = await invoke("is_steam_running");
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
  }

  function updateAltInstallSettings() {
    let newSettings = {};
    
    if (hlaeSettings.launch_options !== defaultHlaeSettings.launch_options) {
      newSettings.launch_options = hlaeSettings.launch_options;
    }
    
    if (hlaeSettings.use_64bit !== defaultHlaeSettings.use_64bit) {
      newSettings.use_64bit = hlaeSettings.use_64bit;
    }
    
    if (hlaeSettings.playdemo !== defaultHlaeSettings.playdemo) {
      newSettings.playdemo = hlaeSettings.playdemo;
    }

    hlaeSettings = defaultHlaeSettings;

    settings.alt_installs[tabIndex - 1] = { ...settings.alt_installs[tabIndex - 1], ...newSettings };
  }

  async function launchOnce() {
    isRunning = true;

    if (tabIndex) {
      updateAltInstallSettings();
    }

    await invoke("save_settings", {
      newSettings: JSON.stringify(settings),
    });

    let res = await invoke("launch_tf2", { demoName: startingDemo, install: install, tab: String(tabIndex) });

    if (res?.status && res?.status === "error") {
      alert(res.message);
    }

    isRunning = false;
  }

  async function batchRecord() {
    isRunning = true;

    if (tabIndex) {
      updateAltInstallSettings();
    }

    await invoke("save_settings", {
      newSettings: JSON.stringify(settings),
    });
    let firstRun = true;

    while (true) {
      let resp = await invoke("batch_record", {
        demoName: startingDemo,
        install: install,
        tab: String(tabIndex),
        firstRun
      });

      firstRun = false;

      if (resp === null || resp.complete) {
        break;
      }

      startingDemo = resp.next_demo;
    }

    isRunning = false;
  }

  onMount(() => {
    loadSettings();
  });

  run(() => {
    if (enabled) {
      loadSettings();
      loadDemos();
      checkSteam();
    }

    if (!enabled) {
      dispatch("close");
    }
  });

  async function tabClicked(e) {
    if (tabIndex) {
      updateAltInstallSettings();

      await invoke("save_settings", {
        newSettings: JSON.stringify(settings),
      });
    }

    tabIndex = e.detail;

    install = [{ tf_folder: settings.tf_folder }, ...settings.alt_installs][
      tabIndex
    ].tf_folder;

    if (tabIndex === 0) {
      hlaeSettings = defaultHlaeSettings;
      return;
    }

    hlaeSettings = {
      ...defaultHlaeSettings,
      ...settings.alt_installs[tabIndex - 1],
    };

    delete hlaeSettings.tf_folder;
    delete hlaeSettings.name;
  }
</script>

{#snippet altInstall(customInstall)}
  <div class="setting">
    <div class="launch_options settings__span">
      <Input
        title="TF2 Launch Options"
        bind:value={hlaeSettings.launch_options}
        color={["pri", "sec", "tert"][tabIndex % 3]}
      />
      <Select
        title="DXLevel"
        color={["pri", "sec", "tert"][tabIndex % 3]}
        bind:value={hlaeSettings.dxlevel}
      >
        <option value={80}>80</option>
        <option value={90}>90</option>
        <option value={95}>95</option>
        <option value={100}>100 (Recommended)</option>
      </Select>
      <Input
        title="Width"
        tooltip={`Tf2 window width.`}
        bind:value={hlaeSettings.width}
        color={["pri", "sec", "tert"][tabIndex % 3]}
      />
      <Input
        title="Height"
        tooltip={`Tf2 window height.`}
        bind:value={hlaeSettings.height}
        color={["pri", "sec", "tert"][tabIndex % 3]}
      />
      <a href="https://docs.comfig.app/9.9.3/customization/launch_options">Learn More about Launch Options and DXLevel</a>
    </div>
    <Switch
      title="64Bit"
      bind:value={hlaeSettings.use_64bit}
      tooltip="Launches with 64Bit TF2."
      color={["pri", "sec", "tert"][tabIndex % 3]}
    />
    <Switch
      title="Automatically playdemo"
      bind:value={hlaeSettings.playdemo}
      tooltip="Plays first demo on list as soon as it launches."
      color={["pri", "sec", "tert"][tabIndex % 3]}
    />
    <Switch
      title="Skip Intro Video"
      bind:value={hlaeSettings.novid}
      tooltip="Uses -novid launch option."
      color={["pri", "sec", "tert"][tabIndex % 3]}
    />
    <Switch
      title="Borderless Window"
      bind:value={hlaeSettings.borderless}
      tooltip="Uses -windowed and -noborder launch options."
      color={["pri", "sec", "tert"][tabIndex % 3]}
    />
    {#if outputSettings.method === "sparklyfx" && !hlaeSettings.use_64bit}
      <div class="settings__span no_margin">
        HLAE will be automatically injected into TF2.
      </div>
    {/if}
  </div>
{/snippet}

{#if !["svr", "svr.mov", "svr.mp4"].includes(outputSettings.method)}
  <button class="btn btn--tert btn__launch" onclick={toggle}>
    <Fa icon={faPlay} color={`var(--tert)`} />
    Launch TF2
  </button>

  <Modal
    color="tert"
    {toggle}
    {enabled}
    tabHeaders={settings.alt_installs.length && isSteamRunning && !isRunning
      ? ["Default", ...settings.alt_installs.map((install) => install.name)]
      : null}
    tabs={settings.alt_installs.length && isSteamRunning && !isRunning
      ? [altInstall, ...settings.alt_installs.map(() => altInstall)]
      : null}
    on:tabClicked={tabClicked}
    min_width="600px"
  >
    {#snippet header()}
      {#if isSteamRunning}
        {#if !isRunning}
          <h1>Launch TF2</h1>

          <div class="setting setting__nomargin">
            <div
              class={`datalist ${settings.alt_installs.length == 0 ? "settings__span" : ""}`}
            >
              <Datalist
                title="Starting Demo"
                bind:value={startingDemo}
                tooltip={`The first demo to record.`}
                color="tert"
                items={demos}
              ></Datalist>
            </div>
            {#if settings.alt_installs.length > 0}
              <Select
                title="TF2 Install"
                bind:value={install}
                tooltip={`The install you want to launch.`}
                color="tert"
                disabled
              >
                <option value={settings.tf_folder}>Default</option>
                {#each settings.alt_installs as install}
                  <option value={install.tf_folder}>{install.name}</option>
                {/each}
              </Select>
            {/if}
            {#if outputSettings.method === "sparklyfx"}
              <Input
                title="HLAE .exe Path"
                bind:value={hlaeSettings.hlae_path}
                color="tert"
              />
              <div>
                <Input
                  title="SparklyFX .dll Path"
                  tooltip="The .dll will be automatically adjusted for 64 vs 32Bit."
                  bind:value={hlaeSettings.sparklyfx_path}
                  color="tert"
                />
              </div>
            {/if}
          </div>
        {/if}
      {/if}
    {/snippet}
    {#if isSteamRunning}
      {#if !isRunning}
        <div class="setting">
          <div class="launch_options settings__span">
            <Input
              title="TF2 Launch Options"
              bind:value={hlaeSettings.launch_options}
              color="tert"
            />
            <Select
              title="DXLevel"
              color="tert"
              bind:value={hlaeSettings.dxlevel}
            >
              <option value={80}>80</option>
              <option value={90}>90</option>
              <option value={95}>95</option>
              <option value={100}>100 (Recommended)</option>
            </Select>
            <Input
              title="Width"
              tooltip={`Tf2 window width.`}
              bind:value={hlaeSettings.width}
              color="tert"
            />
            <Input
              title="Height"
              tooltip={`Tf2 window height.`}
              bind:value={hlaeSettings.height}
              color="tert"
            />
            <a href="https://docs.comfig.app/9.9.3/customization/launch_options">Learn More about Launch Options and DXLevel</a>
          </div>
          <Switch
            title="64Bit"
            bind:value={hlaeSettings.use_64bit}
            tooltip="Launches with 64Bit TF2."
            color="tert"
          />
          <Switch
            title="Automatically playdemo"
            bind:value={hlaeSettings.playdemo}
            tooltip="Plays first demo on list as soon as it launches."
            color="tert"
            left={true}
          />
          <Switch
            title="Skip Intro Video"
            bind:value={hlaeSettings.novid}
            tooltip="Uses -novid launch option."
            color="tert"
          />
          <Switch
            title="Borderless Window"
            bind:value={hlaeSettings.borderless}
            tooltip="Uses -windowed and -noborder launch options."
            color="tert"
            left={true}
          />
          {#if outputSettings.method === "sparklyfx" && !hlaeSettings.use_64bit}
            <div class="settings__span no_margin">
              HLAE will be automatically injected into TF2.
            </div>
          {/if}
        </div>
      {:else}
        <h1>Running Team Fortress 2</h1>
        {#if hlaeSettings.playdemo && startingDemo}
          <p>Playing demo: {startingDemo}</p>
        {/if}
      {/if}
    {:else}
      <h1>Steam is not running</h1>
      <p>Open Steam before launching TF2.</p>
    {/if}

    {#snippet footer()}
      <div class="buttons">
        {#if isSteamRunning}
          <button onclick={toggle} class="cancel-btn">Cancel</button>
          <button onclick={launchOnce} class="btn btn--sec">
            Launch Once
          </button>
          <button
            onclick={batchRecord}
            class="btn btn--pri"
            disabled={outputSettings.method !== "sparklyfx"}
          >
            Batch Record
          </button>
        {/if}
      </div>
    {/snippet}
  </Modal>
{/if}

<style lang="scss">
  .buttons {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 1rem;
    padding-top: 1rem;
  }

  .btn__launch {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .no_margin {
    margin: 0;
  }

  .datalist {
    min-width: 0;
    min-height: 0;
  }

  .setting__nomargin {
    margin: 0;
  }

  .launch_options {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    align-items: center;
    gap: 0.5rem;

    & a {
      grid-column: 1 / span 2;
    }
  }
</style>
