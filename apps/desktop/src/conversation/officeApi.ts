import { invoke } from "@tauri-apps/api/core";

export type OfficeFormat = "docx" | "xlsx" | "pptx";

export type OfficeItem = {
  id: string;
  label: string;
  kind: string;
  text: string;
  clipped: boolean;
  chart?: { categories: string[]; values: string[] };
  cell?: { valueType: string; formula: string | null; cache: string | null; numberFormat: string | null; hidden: boolean };
};

export type OfficeView = {
  format: OfficeFormat;
  version: string;
  sections: Array<{ id: string; label: string; kind: string }>;
  section: string;
  items: OfficeItem[];
  offset: number;
  total: number;
  nextOffset: number | null;
  readOnly: string | null;
};

export type OfficeSourceTarget =
  | { kind: "project"; project: string; path: string }
  | { kind: "attachment"; path: string; session?: string }
  | { kind: "result"; id: string };

export type OfficePanelTarget =
  | OfficeSourceTarget
  | { kind: "drafts"; session: string; turn: number }
  | { kind: "pick" };

export type OpenedOffice = { handle: string; name: string; sourcePath: string; view: OfficeView };
export type OfficeDraft = {
  handle: string;
  name: string;
  format: OfficeFormat;
  changes: Array<{ section: string; item: string; before: string; after: string }>;
  needsCalculation: boolean;
  needsPreview: boolean;
};
export type OfficeSaved = { path: string; resultId: string | null; registrationError: string | null };
export type OfficePreview = { image: string; pages: number; version: string };

export const supportsOffice = (path: string) => /\.(docx|xlsx|pptx)$/i.test(path);
export const pickOffice = () => invoke<OpenedOffice | null>("office_pick");

export function openOffice(target: OfficeSourceTarget) {
  switch (target.kind) {
    case "project":
      return invoke<OpenedOffice>("office_open_project", { project: target.project, path: target.path });
    case "attachment":
      return invoke<OpenedOffice>("office_open_attachment", { path: target.path, session: target.session ?? null });
    case "result":
      return invoke<OpenedOffice>("office_open_result", { id: target.id });
  }
}

export const readOffice = (handle: string, draft: boolean, section: string | null, offset: number, range: string) =>
  invoke<OfficeView>("office_view", { handle, draft, section, offset, range: range || null });
export const renderOffice = (handle: string, draft: boolean, page: number) =>
  invoke<OfficePreview>("office_render", { handle, draft, page });
export const saveOffice = (handle: string, saveAsDraft: boolean) =>
  invoke<OfficeSaved | null>("office_save", { handle, saveAsDraft });
export const openExternalOffice = (handle: string) => invoke<void>("office_open_external", { handle });
export const closeOffice = (handle: string) => invoke<void>("office_close", { handle });
export const listOfficeDrafts = (session: string, turn: number) => invoke<OfficeDraft[]>("office_pending_drafts", { session, turn });
