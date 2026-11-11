import { bindAnalysisEvents, syncSettings } from "./analysis.js";
import { bindMappingEvents } from "./mapping.js";
import { bindResultsEvents, bindResultsSplitter } from "./results.js";
import { bindSourceEvents } from "./source.js";
import { bindNavigation } from "./ui.js";
import { initializeCollaboration } from "./collaboration.js";
import { bindGuidedFlow } from "./guided.js";

document.addEventListener("DOMContentLoaded", () => {
  bindNavigation();
  bindSourceEvents();
  bindMappingEvents();
  bindAnalysisEvents();
  bindResultsEvents();
  bindResultsSplitter();
  bindGuidedFlow();
  syncSettings();
  initializeCollaboration().catch(error=>console.error(error));
});
