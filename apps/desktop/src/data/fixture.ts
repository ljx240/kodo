import raw from "../../../../demo/conversation.fixture.json";
import type { Fixture } from "./types";

/** The deterministic demo fixture required by docs/design/DEMO_DATA.md. */
export const fixture = raw as Fixture;

export const project = fixture.project;
export const projects = fixture.projects;
export const conversation = fixture.conversation;
export const changedFiles = fixture.changed_files;
export const summary = fixture.summary;
