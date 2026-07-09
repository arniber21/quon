import * as fs from "fs";
import * as path from "path";
import Mocha from "mocha";

function collectTests(dir: string, acc: string[] = []): string[] {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      collectTests(full, acc);
    } else if (entry.isFile() && entry.name.endsWith(".test.js")) {
      acc.push(full);
    }
  }
  return acc;
}

export function run(): Promise<void> {
  const mocha = new Mocha({
    ui: "tdd",
    color: true,
    timeout: 60_000,
  });

  const testsRoot = path.resolve(__dirname);
  for (const file of collectTests(testsRoot)) {
    mocha.addFile(file);
  }

  return new Promise((resolve, reject) => {
    mocha.run((failures) => {
      if (failures > 0) {
        reject(new Error(`${failures} tests failed.`));
      } else {
        resolve();
      }
    });
  });
}
