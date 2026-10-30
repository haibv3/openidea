import { describe, expect, it } from "vitest";
import type { PluginInfo, PluginPreviewDecl } from "@/bindings";
import { findPreviewProvider, pathMatchesGlob } from "./match";

function decl(partial: Partial<PluginPreviewDecl> & Pick<PluginPreviewDecl, "id">): PluginPreviewDecl {
  return { title: partial.id, extensions: [], pathGlobs: [], ...partial };
}

function plugin(partial: Partial<PluginInfo> & Pick<PluginInfo, "id">): PluginInfo {
  return {
    name: `Plugin ${partial.id}`,
    version: "1.0.0",
    description: "",
    authors: [],
    homepage: null,
    changeNotes: null,
    source: "installed",
    path: `/plugins/${partial.id}`,
    enabled: true,
    status: "active",
    error: null,
    hasCode: true,
    permissions: [],
    commands: [],
    languages: [],
    themes: [],
    toolWindows: [],
    previews: [],
    ...partial,
  };
}

describe("pathMatchesGlob", () => {
  const cases: ReadonlyArray<[glob: string, path: string, matches: boolean]> = [
    ["**/res/layout/**", "res/layout/main.xml", true],
    ["**/res/layout/**", "app/src/main/res/layout/main.xml", true],
    ["**/res/layout/**", "app/src/main/res/layout/sub/inner.xml", true],
    ["**/res/layout/**", "res/layoutx/main.xml", false],
    ["**/res/layout/**", "main.xml", false],
    ["**/res/layout/**", "app/src/main/res/layout", false],
    ["src/*.ts", "src/a.ts", true],
    ["src/*.ts", "src/sub/a.ts", false], // * stays within one segment
    ["src/*.ts", "src/a.tsx", false],
    ["*.xml", "a/b.xml", false],
    ["layout?.xml", "layout1.xml", true], // ? is one character
    ["layout?.xml", "layout12.xml", false],
    ["layout?.xml", "layout.xml", false],
    ["a/**/b", "a/b", true], // ** also spans no directories
    ["a/**/b", "a/x/y/b", true],
    ["**", "any/thing.xml", true],
    ["res/**", "RES/layout.xml", false], // case-sensitive
    ["res/**", "res\\layout.xml", true], // backslashes normalized
  ];
  for (const [glob, path, ok] of cases) {
    it(`${ok ? "matches" : "rejects"} ${path} against ${glob}`, () => {
      expect(pathMatchesGlob(path, glob)).toBe(ok);
    });
  }
});

describe("findPreviewProvider", () => {
  const layout = (over: Partial<PluginPreviewDecl> = {}) =>
    decl({ id: "layout", title: "Layout Preview", extensions: ["xml"], pathGlobs: ["**/res/layout/**"], ...over });

  it("returns the first active plugin and declaration covering the file", () => {
    const p = plugin({ id: "layouter", previews: [layout()] });
    expect(findPreviewProvider("app/src/main/res/layout/main.xml", [p])).toEqual({
      pluginId: "layouter",
      pluginName: "Plugin layouter",
      pluginTitle: "Plugin layouter",
      declId: "layout",
      title: "Layout Preview",
    });
  });

  it("prefers the first plugin, then the first declaration, in order", () => {
    const plugins = [
      plugin({ id: "other", previews: [decl({ id: "never", pathGlobs: ["nomatch/**"] })] }),
      plugin({ id: "first", previews: [decl({ id: "nope", pathGlobs: ["nomatch/**"] }), layout()] }),
      plugin({ id: "second", previews: [layout()] }),
    ];
    expect(findPreviewProvider("res/layout/main.xml", plugins)?.declId).toBe("layout");
    expect(findPreviewProvider("res/layout/main.xml", plugins)?.pluginId).toBe("first");
  });

  const cases: ReadonlyArray<[name: string, plugins: PluginInfo[], path: string, root: string | null]> = [
    ["inactive plugins are skipped", [plugin({ id: "p", status: "disabled", previews: [layout()] })], "res/a.xml", null],
    ["static plugins are skipped", [plugin({ id: "p", status: "static", previews: [layout()] })], "res/a.xml", null],
    ["plugins without previews are skipped", [plugin({ id: "p", previews: [] })], "res/a.xml", null],
    ["empty plugin list", [], "res/a.xml", null],
    [
      "extension compare is case-insensitive",
      [plugin({ id: "p", previews: [layout()] })],
      "res/layout/MAIN.XML",
      null,
    ],
    [
      "extension must match when both are set",
      [plugin({ id: "p", previews: [layout()] })],
      "res/layout/main.txt",
      null,
    ],
    ["globs must match when both are set", [plugin({ id: "p", previews: [layout()] })], "other/main.xml", null],
    [
      "extensions alone decide when globs are empty",
      [plugin({ id: "p", previews: [decl({ id: "d", extensions: ["xml"] })] })],
      "deeply/nested/thing.xml",
      null,
    ],
    [
      "globs alone decide when extensions are empty",
      [plugin({ id: "p", previews: [decl({ id: "d", pathGlobs: ["docs/*.md"] })] })],
      "docs/README.md",
      null,
    ],
    [
      "globs match the project-relative path",
      [plugin({ id: "p", previews: [layout()] })],
      "D:/proj/app/src/main/res/layout/main.xml",
      "D:/proj",
    ],
    [
      "project root with a trailing slash",
      [plugin({ id: "p", previews: [layout()] })],
      "D:\\proj\\app\\res\\layout\\main.xml",
      "D:/proj/",
    ],
    [
      "full path tried when the relative match fails",
      [plugin({ id: "p", previews: [decl({ id: "d", pathGlobs: ["proj/**"] })] })],
      "D:/proj/a.xml",
      "D:/proj",
    ],
    [
      "paths outside the project match against the full path",
      [plugin({ id: "p", previews: [decl({ id: "d", pathGlobs: ["**/a.xml"] })] })],
      "D:/other/a.xml",
      "D:/proj",
    ],
  ];
  for (const [name, plugins, path, root] of cases) {
    it(name, () => {
      const want = /skipped|empty plugin list/.test(name) ? null : expect.any(Object);
      expect(findPreviewProvider(path, plugins, root)).toEqual(want);
    });
  }

  it("returns null when nothing matches", () => {
    expect(findPreviewProvider("res/main.json", [plugin({ id: "p", previews: [layout()] })], null)).toBeNull();
  });
});
