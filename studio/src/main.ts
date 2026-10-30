import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

type Paths = {
  root: string;
  out: string;
  sample: string;
  game_default: string;
  install_cd: string;
  bootstrap: string;
  fixpack_guide_url: string;
  patch_105_url: string;
};

type GameStatus = {
  game_dir: string;
  has_exe: boolean;
  localized_lang: string | null;
  safedisc: boolean;
  has_patch_105_installer: boolean;
  has_d3d8: boolean;
  has_d3d9: boolean;
  has_launcher: boolean;
  has_fixpack_marker: boolean;
  has_hires_data: boolean;
  has_asi_loader: boolean;
  has_mod_data: boolean;
  graphics_stack_ok: boolean;
  summary: string;
  next_step: string;
};

const logEl = () => document.querySelector<HTMLPreElement>("#log")!;
const $ = <T extends HTMLElement>(sel: string) => document.querySelector<T>(sel)!;
const input = (sel: string) => $(sel) as HTMLInputElement;

function stamp(msg: string) {
  const t = new Date().toLocaleTimeString();
  logEl().textContent += `[${t}] ${msg}\n`;
  logEl().scrollTop = logEl().scrollHeight;
}

async function withBusy(btns: HTMLButtonElement[], fn: () => Promise<void>) {
  btns.forEach((b) => (b.disabled = true));
  try {
    await fn();
  } finally {
    btns.forEach((b) => (b.disabled = false));
  }
}

function wireTabs() {
  document.querySelectorAll<HTMLButtonElement>(".tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      document.querySelectorAll(".tab").forEach((t) => t.classList.remove("active"));
      document.querySelectorAll(".panel").forEach((p) => p.classList.remove("active"));
      tab.classList.add("active");
      $(`#panel-${tab.dataset.tab}`).classList.add("active");
    });
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  wireTabs();
  let paths: Paths;
  try {
    paths = await invoke<Paths>("get_paths");
  } catch (e) {
    stamp(`ERROR: ${e}`);
    return;
  }

  input("#archive").value = paths.sample;
  input("#game-dir").value = paths.game_default;
  input("#pack-folder").value = `${paths.out}/sample-mod`;
  input("#pack-out").value = `${paths.root}/my-mod.mpm`;
  stamp(`Repo: ${paths.root}`);

  $("#btn-sample").onclick = () => {
    if (!paths.sample) {
      stamp("Sample missing (vendor/fixtures).");
      return;
    }
    input("#archive").value = paths.sample;
    stamp("Loaded sample mod path.");
  };

  $("#btn-browse-archive").onclick = async () => {
    const f = await open({
      multiple: false,
      filters: [{ name: "Max Payne archives", extensions: ["ras", "mpm", "RAS", "MPM"] }],
    });
    if (typeof f === "string") input("#archive").value = f;
  };

  $("#btn-browse-pack").onclick = async () => {
    const d = await open({ directory: true, multiple: false });
    if (typeof d === "string") input("#pack-folder").value = d;
  };

  $("#btn-browse-cd").onclick = async () => {
    const d = await open({ directory: true, multiple: false });
    if (typeof d === "string") input("#cd-root").value = d;
  };

  $("#btn-browse-game").onclick = async () => {
    const d = await open({ directory: true, multiple: false });
    if (typeof d === "string") input("#game-dir").value = d;
  };

  const listBtn = $("#btn-list") as HTMLButtonElement;
  listBtn.onclick = () =>
    withBusy([listBtn], async () => {
      stamp(`Listing ${input("#archive").value}…`);
      try {
        stamp(await invoke<string>("list_archive", { path: input("#archive").value }));
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });

  const extractBtn = $("#btn-extract") as HTMLButtonElement;
  extractBtn.onclick = () =>
    withBusy([extractBtn], async () => {
      stamp(`Extracting…`);
      try {
        stamp(await invoke<string>("extract_archive", { path: input("#archive").value }));
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });

  $("#btn-open-out").onclick = async () => {
    try {
      await invoke("open_in_file_manager", { path: paths.out });
    } catch (e) {
      stamp(`ERROR: ${e}`);
    }
  };

  const packBtn = $("#btn-pack") as HTMLButtonElement;
  packBtn.onclick = () =>
    withBusy([packBtn], async () => {
      stamp("Packing…");
      try {
        stamp(
          await invoke<string>("pack_archive", {
            folder: input("#pack-folder").value,
            out: input("#pack-out").value,
          }),
        );
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });

  async function refreshStatus() {
    const st = await invoke<GameStatus>("game_status", {
      dest: input("#game-dir").value,
    });
    const box = $("#game-status");
    box.textContent = [
      `Folder: ${st.game_dir}`,
      `MaxPayne.exe: ${st.has_exe ? "yes" : "no"}`,
      `Language pack: ${st.localized_lang ? `x_${st.localized_lang}.ras` : "english / none"}`,
      `SafeDisc stub (stxt*): ${st.safedisc ? "YES — blocked" : "no"}`,
      `Hi-res FixPack data/: ${st.has_hires_data ? "YES (park on localized)" : "no"}`,
      `ASI dinput loader: ${st.has_asi_loader ? "YES (park on localized)" : "no"}`,
      `Loose mod data/: ${st.has_mod_data ? "yes" : "no"}`,
      `Graphics stack: ${st.graphics_stack_ok ? "OK" : "NEEDS FIX"}`,
      `1.05 file: ${st.has_patch_105_installer ? "yes" : "missing"}`,
      `d3d8 / d3d9: ${st.has_d3d8 ? "yes" : "no"} / ${st.has_d3d9 ? "yes" : "no"}`,
      `Launcher: ${st.has_launcher ? "yes" : "no"}`,
      "",
      st.summary,
      "",
      `Next: ${st.next_step}`,
    ].join("\n");
    stamp(st.summary);
    return st;
  }

  $("#btn-find-cd").onclick = async () => {
    try {
      const cd = await invoke<string>("find_cd");
      input("#cd-root").value = cd;
      stamp(`CD: ${cd}`);
    } catch (e) {
      stamp(`No CD found (${e}). Insert the disc and try again.`);
    }
  };

  const installBtn = $("#btn-install") as HTMLButtonElement;
  installBtn.onclick = () =>
    withBusy([installBtn], async () => {
      stamp("Installing from CD (can take several minutes)…");
      try {
        stamp(
          await invoke<string>("install_from_cd", {
            cd: input("#cd-root").value,
            dest: input("#game-dir").value,
          }),
        );
        await refreshStatus();
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });

  const setupBtn = $("#btn-setup") as HTMLButtonElement;
  setupBtn.onclick = () =>
    withBusy([setupBtn], async () => {
      stamp("Installing graphics helpers…");
      try {
        stamp(await invoke<string>("setup_game", { dest: input("#game-dir").value }));
        await refreshStatus();
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });

  $("#btn-status").onclick = () =>
    withBusy([$("#btn-status") as HTMLButtonElement], async () => {
      try {
        await refreshStatus();
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });

  const patchBtn = $("#btn-patch-105") as HTMLButtonElement;
  patchBtn.onclick = () =>
    withBusy([patchBtn], async () => {
      stamp("1.05: download if needed, then replace MaxPayne.exe (not “run as game”)…");
      try {
        stamp(await invoke<string>("run_patch_105", { dest: input("#game-dir").value }));
        await refreshStatus();
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });

  const localizedBtn = $("#btn-localized-safe") as HTMLButtonElement;
  localizedBtn.onclick = () =>
    withBusy([localizedBtn], async () => {
      stamp("Applying localized-safe Wine stack…");
      try {
        stamp(
          await invoke<string>("apply_localized_safe_stack", {
            dest: input("#game-dir").value,
          }),
        );
        await refreshStatus();
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });

  const openGuide = async () => {
    try {
      await invoke("open_url", { url: paths.fixpack_guide_url });
      stamp(`Opened FixPack guide: ${paths.fixpack_guide_url}`);
      stamp("Download the notex build; for retail CD pick CD exe + Startup fix + d3d8to9.");
    } catch (e) {
      stamp(`ERROR: ${e}`);
    }
  };
  $("#btn-fixpack-guide").onclick = () => void openGuide();
  $("#link-fixpack-guide").onclick = (ev) => {
    ev.preventDefault();
    void openGuide();
  };

  $("#btn-fixpack-run").onclick = async () => {
    const f = await open({
      multiple: false,
      filters: [{ name: "FixPack installer", extensions: ["exe"] }],
    });
    if (typeof f !== "string") return;
    const btn = $("#btn-fixpack-run") as HTMLButtonElement;
    await withBusy([btn], async () => {
      stamp(`Running FixPack: ${f}`);
      try {
        stamp(
          await invoke<string>("run_fixpack_installer", {
            dest: input("#game-dir").value,
            installer: f,
          }),
        );
        stamp("In FixPack: retail CD → CD executable, Startup fix, d3d8to9. Then Refresh status.");
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });
  };

  $("#btn-launch").onclick = async () => {
    try {
      const st = await refreshStatus();
      if (st.safedisc) {
        stamp("Blocked: SafeDisc stub still present. Apply 1.05 + FixPack CD exe first.");
        return;
      }
      stamp(await invoke<string>("launch_game", { dest: input("#game-dir").value }));
    } catch (e) {
      stamp(`ERROR: ${e}`);
    }
  };

  $("#btn-open-game-dir").onclick = async () => {
    try {
      await invoke("open_in_file_manager", { path: input("#game-dir").value });
    } catch (e) {
      stamp(`ERROR: ${e}`);
    }
  };

  try {
    await refreshStatus();
  } catch (e) {
    stamp(`Status: ${e}`);
  }

  const testBtn = $("#btn-tests") as HTMLButtonElement;
  testBtn.onclick = () =>
    withBusy([testBtn], async () => {
      stamp("Running tests…");
      try {
        stamp(await invoke<string>("run_tests"));
      } catch (e) {
        stamp(`ERROR: ${e}`);
      }
    });

});
