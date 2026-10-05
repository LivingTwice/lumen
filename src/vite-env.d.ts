/// <reference types="vite/client" />

// version allégée de hls.js (aperçu des vidéos sous Windows) : mêmes types que la version complète
declare module "hls.js/light" {
  export { default } from "hls.js";
}
