import { createApp } from 'vue'
import { createPinia } from 'pinia'
import {
  create, NAlert, NButton, NCard, NCheckbox, NCollapse, NCollapseItem,
  NCollapseTransition, NConfigProvider, NDataTable, NDrawer, NDrawerContent,
  NDropdown, NForm, NFormItem, NGi, NGrid, NImage, NInput, NInputNumber,
  NLayout, NLayoutContent, NLayoutHeader, NLayoutSider, NMenu, NMessageProvider,
  NModal, NPagination, NPopconfirm, NSelect, NSkeleton, NSpace, NSpin, NSwitch,
  NTab, NTabPane, NTabs, NTag, NTimePicker, NTimeline, NTimelineItem,
} from 'naive-ui'
import App from './App.vue'
import router from './router'
import './styles.css'

// Register only components used by the panel so unused UI modules can be removed.
const ui = create({ components: [
  NAlert, NButton, NCard, NCheckbox, NCollapse, NCollapseItem, NCollapseTransition,
  NConfigProvider, NDataTable, NDrawer, NDrawerContent, NDropdown, NForm, NFormItem,
  NGi, NGrid, NImage, NInput, NInputNumber, NLayout, NLayoutContent, NLayoutHeader,
  NLayoutSider, NMenu, NMessageProvider, NModal, NPagination, NPopconfirm, NSelect,
  NSkeleton, NSpace, NSpin, NSwitch, NTab, NTabPane, NTabs, NTag, NTimePicker,
  NTimeline, NTimelineItem,
] })
createApp(App).use(createPinia()).use(router).use(ui).mount('#app')
