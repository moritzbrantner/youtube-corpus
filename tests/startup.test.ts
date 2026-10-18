import { afterEach, expect, test } from "bun:test";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { randomUUID } from "node:crypto";

const root = resolve(import.meta.dirname, "..");
const temporary: string[] = [];
function temp() {
  const directory = mkdtempSync(join(tmpdir(), "youtube-corpus-startup-"));
  temporary.push(directory);
  return directory;
}
afterEach(() => {
  for (const directory of temporary.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function candidateClone() {
  const checkout = join(temp(), "checkout");
  execFileSync("git", ["clone", "--quiet", "--no-hardlinks", root, checkout]);
  const diff = execFileSync("git", ["diff", "HEAD", "--binary"], { cwd: root });
  if (diff.length > 0) {
    execFileSync("git", ["apply", "--binary", "-"], { cwd: checkout, input: diff });
  }
  return checkout;
}

function diagnostic(path: string, env: NodeJS.ProcessEnv) {
  try {
    execFileSync("/bin/bash", [join(path, "scripts/start.sh")], { cwd: path, env, stdio: "pipe" });
    throw new Error("startup should fail");
  } catch (error) {
    if (
      typeof error === "object" &&
      error !== null &&
      "stderr" in error &&
      Buffer.isBuffer(error.stderr)
    ) {
      return error.stderr.toString();
    }
    throw error;
  }
}

test("missing Bun has an actionable diagnostic before package/build work", () => {
  const missing = join(temp(), "empty");
  expect(diagnostic(root, { ...process.env, PATH: missing })).toContain(
    "Bun is required. Install Bun",
  );
});

test("missing Docker and Podman name the local Postgres prerequisite", () => {
  const binaries = temp();
  for (const command of ["bun", "cargo", "cp"]) {
    const location = execFileSync("/bin/bash", ["-c", `command -v ${command}`], {
      encoding: "utf8",
    }).trim();
    symlinkSync(location, join(binaries, command));
  }
  // The wrapper is deliberately invoked from its own checkout, as documented.
  // An explicit default URL overrides any external DATABASE_URL inherited by this test.
  expect(
    diagnostic(candidateClone(), {
      ...process.env,
      PATH: binaries,
      DATABASE_URL: "postgres://postgres:postgres@localhost:5432/youtube_corpus",
    }),
  ).toContain("Docker Compose or Podman Compose is required for local Postgres");
});

async function stop(child: ChildProcess) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return;
  }
  const exited = once(child, "exit");
  child.kill("SIGTERM");
  const [code] = await exited;
  expect(code).toBe(143);
}

async function start(checkout: string, env: NodeJS.ProcessEnv) {
  const child = spawn("/bin/bash", ["scripts/start.sh", "--port", "0", "--no-open"], {
    cwd: checkout,
    env,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let logs = "";
  try {
    const url = await new Promise<string>((resolveReady, reject) => {
      const timeout = setTimeout(() => reject(new Error(`Startup timeout:\n${logs}`)), 240000);
      const collect = (chunk: Buffer) => {
        logs += chunk.toString();
        const match = logs.match(/serving YouTube Corpus at (http:\/\/127\.0\.0\.1:\d+)/);
        if (match?.[1]) {
          clearTimeout(timeout);
          resolveReady(match[1]);
        }
      };
      child.stdout?.on("data", collect);
      child.stderr?.on("data", collect);
      child.once("error", (error) => {
        clearTimeout(timeout);
        reject(error);
      });
      child.once("exit", (code) => {
        clearTimeout(timeout);
        reject(new Error(`Startup exited ${code}:\n${logs}`));
      });
    });
    return { child, url, logs: () => logs };
  } catch (error) {
    await stop(child);
    throw error;
  }
}

test("fresh clone starts on Compose Postgres and repeated start preserves configuration, schema and frontend", async () => {
  execFileSync("docker", ["compose", "up", "-d", "--wait", "postgres"], {
    cwd: root,
    stdio: "pipe",
  });
  const database = `youtube_corpus_startup_${randomUUID().replaceAll("-", "").slice(0, 12)}`;
  const databaseUrl = `postgres://postgres:postgres@localhost:5432/${database}`;
  const postgres = (args: string[]) =>
    execFileSync("docker", ["compose", "exec", "-T", "postgres", ...args], {
      cwd: root,
      encoding: "utf8",
    });
  postgres(["createdb", "-U", "postgres", database]);
  const checkout = candidateClone();
  const probe = join(temp(), "media-commands");
  const binaries = temp();
  for (const command of ["yt-dlp", "whisper"]) {
    writeFileSync(
      join(binaries, command),
      '#!/bin/sh\necho invoked >> "$STARTUP_MEDIA_PROBE"\nexit 91\n',
      { mode: 0o755 },
    );
  }
  const env = {
    ...process.env,
    DATABASE_URL: databaseUrl,
    CARGO_TARGET_DIR: join(root, "target"),
    PATH: `${binaries}:${process.env.PATH ?? ""}`,
    STARTUP_MEDIA_PROBE: probe,
  };
  let server: Awaited<ReturnType<typeof start>> | undefined;
  try {
    server = await start(checkout, env);
    const response = await fetch(server.url);
    expect(response.status).toBe(200);
    expect(await response.text()).toContain("youtube-corpus");
    const sql = (query: string) =>
      postgres(["psql", "-U", "postgres", "-d", database, "-At", "-c", query]).trim();
    expect(Number(sql("SELECT count(*) FROM _sqlx_migrations WHERE success"))).toBe(
      readdirSync(join(root, "migrations")).filter((file) => file.endsWith(".sql")).length,
    );
    expect(sql("SELECT to_regclass('detector_attempts')")).toBe("detector_attempts");
    expect(sql("SELECT count(*) FROM videos")).toBe("0");
    expect(sql("SELECT count(*) FROM ingest_runs")).toBe("0");
    await stop(server.child);
    server = undefined;
    const envPath = join(checkout, ".env");
    expect(existsSync(envPath)).toBe(true);
    writeFileSync(envPath, readFileSync(envPath, "utf8") + "\n# retain my configuration\n");
    const contents = readFileSync(envPath, "utf8");
    const envTime = statSync(envPath).mtimeMs;
    const index = join(checkout, "dist/index.html");
    const indexTime = statSync(index).mtimeMs;
    const schemaCount = sql("SELECT count(*) FROM _sqlx_migrations");
    server = await start(checkout, env);
    expect((await fetch(server.url)).status).toBe(200);
    expect(server.logs()).toContain("Frontend is already built for these inputs.");
    expect(readFileSync(envPath, "utf8")).toBe(contents);
    expect(statSync(envPath).mtimeMs).toBe(envTime);
    expect(statSync(index).mtimeMs).toBe(indexTime);
    expect(sql("SELECT count(*) FROM _sqlx_migrations")).toBe(schemaCount);
    expect(sql("SELECT count(*) FROM videos")).toBe("0");
    expect(existsSync(probe)).toBe(false);
    expect(existsSync(join(checkout, "use-case-output/models"))).toBe(false);
  } finally {
    if (server) {
      await stop(server.child);
    }
    postgres(["dropdb", "-U", "postgres", database]);
  }
}, 300000);
