import { createHash } from "node:crypto";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import {
  existsSync,
  readdirSync,
  readFileSync,
  renameSync,
  mkdirSync,
  writeFileSync,
} from "node:fs";
import { createConnection } from "node:net";
import { join, relative, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
process.chdir(root);
const defaultDatabase = "postgres://postgres:postgres@localhost:5432/youtube_corpus";
let activeChild: ChildProcess | undefined;
let interrupted: NodeJS.Signals | undefined;
for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.once(signal, () => {
    interrupted = signal;
    activeChild?.kill(signal);
  });
}

function available(command: string, args = ["--version"]) {
  return spawnSync(command, args, { stdio: "ignore" }).status === 0;
}

async function run(command: string, args: string[], diagnostic: string) {
  if (interrupted) {
    throw new Error("Startup interrupted.");
  }
  const child = spawn(command, args, { stdio: "inherit" });
  activeChild = child;
  try {
    const code = await new Promise<number | null>((resolveExit, reject) => {
      child.once("error", reject);
      child.once("exit", resolveExit);
    });
    if (code !== 0 && !interrupted) {
      throw new Error(diagnostic);
    }
  } finally {
    activeChild = undefined;
  }
}

function filesUnder(directory: string): string[] {
  if (!existsSync(directory)) {
    return [];
  }
  return readdirSync(directory, { withFileTypes: true })
    .sort((a, b) => a.name.localeCompare(b.name, "en"))
    .flatMap((entry) => {
      const path = join(directory, entry.name);
      const name = relative(root, path).replaceAll("\\", "/");
      if (
        entry.isSymbolicLink() ||
        ["target", "node_modules", ".git", "dist", "use-case-output", ".cargo", ".sqlx"].includes(
          entry.name,
        ) ||
        entry.name === "youtube-corpus.conf" ||
        entry.name === ".env" ||
        name === "src/app/features/analyzer/wasm"
      ) {
        return [];
      }
      return entry.isDirectory() ? filesUnder(path) : [path];
    });
}

function fingerprint(files: string[], tools = "") {
  const hash = createHash("sha256");
  hash.update(tools);
  for (const file of files) {
    const bytes = readFileSync(file);
    hash.update(relative(root, file).replaceAll("\\", "/") + "\0" + bytes.length + "\0");
    hash.update(bytes);
  }
  return hash.digest("hex");
}

function toolVersion(command: string) {
  const result = spawnSync(command, ["--version"], { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(
      `Could not run ${command}. Repair its installation and rerun ./scripts/start.sh.`,
    );
  }
  return result.stdout.trim();
}

async function prepareFrontend() {
  await run(
    "bun",
    ["install", "--frozen-lockfile"],
    "Frontend dependencies could not be installed. Check registry access and bun.lock, then retry.",
  );
  if (!available("wasm-pack")) {
    await run(
      "cargo",
      ["install", "wasm-pack", "--version", "0.14.0", "--locked"],
      "wasm-pack could not be installed. Run cargo install wasm-pack --version 0.14.0 --locked, then retry.",
    );
  }
  // Tailwind scans repository text as well as frontend modules. Include owned source,
  // docs and configuration while excluding local credentials and generated outputs.
  const inputs = filesUnder(root);
  const source = fingerprint(
    inputs,
    [toolVersion("bun"), toolVersion("wasm-pack"), toolVersion("rustc")].join("\n"),
  );
  const receipt = join(root, "use-case-output/startup/frontend.json");
  const outputs = filesUnder(join(root, "dist"));
  let cached: unknown;
  if (existsSync(receipt)) {
    try {
      cached = JSON.parse(readFileSync(receipt, "utf8"));
    } catch {
      // A corrupt disposable receipt is a cache miss, never proof that a build passed.
      cached = undefined;
    }
  }
  if (
    outputs.length > 0 &&
    existsSync(join(root, "dist/index.html")) &&
    typeof cached === "object" &&
    cached !== null &&
    "source" in cached &&
    "output" in cached &&
    cached.source === source &&
    cached.output === fingerprint(outputs)
  ) {
    console.log("Frontend is already built for these inputs.");
    return;
  }
  await run(
    "bun",
    ["run", "build"],
    "Frontend build failed. Fix the reported build error and rerun ./scripts/start.sh.",
  );
  mkdirSync(join(root, "use-case-output/startup"), { recursive: true });
  const temporary = receipt + `.${process.pid}.tmp`;
  writeFileSync(
    temporary,
    JSON.stringify({ source, output: fingerprint(filesUnder(join(root, "dist"))) }) + "\n",
  );
  renameSync(temporary, receipt);
}

async function checkDatabasePort(database: URL) {
  const host = database.hostname.replace(/^\[|\]$/g, "");
  const port = Number(database.port || 5432);
  await new Promise<void>((resolvePort, reject) => {
    const socket = createConnection({ host, port });
    const fail = () => {
      socket.destroy();
      reject(
        new Error(
          `Postgres is unavailable at ${host}:${port}. Start Postgres or correct DATABASE_URL in .env.`,
        ),
      );
    };
    socket.setTimeout(5000, fail);
    socket.once("error", fail);
    socket.once("connect", () => {
      socket.destroy();
      resolvePort();
    });
  });
}

async function main() {
  if (!available("cargo")) {
    throw new Error(
      "Cargo is required. Install Rust stable with rustup, then rerun ./scripts/start.sh.",
    );
  }
  const args = process.argv.slice(2).filter((argument, index) => index !== 0 || argument !== "--");
  // Explicit startup flags override .env, just as explicit CLI flags override its config file.
  let databaseUrl = process.env.DATABASE_URL || defaultDatabase;
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === "--database-url") {
      const value = args[index + 1];
      if (!value) {
        throw new Error("--database-url requires a Postgres URL.");
      }
      databaseUrl = value;
    } else if (argument?.startsWith("--database-url=")) {
      databaseUrl = argument.slice("--database-url=".length);
    }
  }
  let database: URL;
  try {
    database = new URL(databaseUrl);
  } catch {
    throw new Error(
      "DATABASE_URL must be a valid postgres:// URL. Correct .env or pass --database-url.",
    );
  }
  if (!["postgres:", "postgresql:"].includes(database.protocol)) {
    throw new Error(
      "DATABASE_URL must use postgres:// or postgresql://. Correct .env or pass --database-url.",
    );
  }
  process.env.DATABASE_URL = databaseUrl;
  if (databaseUrl === defaultDatabase) {
    let engine: string;
    if (available("docker", ["compose", "version"])) {
      engine = "docker";
    } else if (available("podman", ["compose", "version"])) {
      engine = "podman";
    } else {
      throw new Error(
        "Docker Compose or Podman Compose is required for local Postgres. Install/start one, or set DATABASE_URL in .env to an existing Postgres server.",
      );
    }
    await run(
      engine,
      ["compose", "up", "-d", "--wait", "postgres"],
      "Postgres could not start. Make sure the container engine is running and port 5432 is free, or set DATABASE_URL in .env to an existing Postgres server.",
    );
  }
  await checkDatabasePort(database);
  await prepareFrontend();
  await run(
    "cargo",
    ["build", "--locked"],
    "Rust build failed. Fix the reported compiler/dependency error, then retry.",
  );
  const metadata = spawnSync(
    "cargo",
    ["metadata", "--locked", "--no-deps", "--format-version", "1"],
    {
      encoding: "utf8",
    },
  );
  const parsed: unknown = JSON.parse(metadata.stdout || "null");
  if (
    metadata.status !== 0 ||
    typeof parsed !== "object" ||
    parsed === null ||
    !("target_directory" in parsed) ||
    typeof parsed.target_directory !== "string"
  ) {
    throw new Error(
      "Cargo could not locate its build directory. Run cargo metadata --no-deps --format-version 1 to inspect the error.",
    );
  }
  const binary = join(
    parsed.target_directory,
    "debug",
    process.platform === "win32" ? "youtube-corpus.exe" : "youtube-corpus",
  );
  await run(
    binary,
    ["--database-url", databaseUrl, "migrate"],
    "Postgres migration failed. Check DATABASE_URL credentials, database existence and pgvector support, then retry.",
  );
  if (!interrupted) {
    await run(
      binary,
      ["--database-url", databaseUrl, ...args],
      "The server could not start. If port 1420 is occupied, stop the existing server or pass --port 1421.",
    );
  }
}

try {
  await main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
} finally {
  if (interrupted) {
    process.exitCode = interrupted === "SIGINT" ? 130 : 143;
  }
}
