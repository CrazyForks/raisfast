import { Routes, Route, Navigate } from "react-router-dom";
import { Providers } from "@/components/providers";
import AdminLayout from "@/pages/layout";
import LoginPage from "@/pages/login";
import DashboardPage from "@/pages/dashboard";
import PostsPage from "@/pages/posts";
import PostsNewPage from "@/pages/posts-new";
import PostsEditPage from "@/pages/posts-edit";
import CategoriesPage from "@/pages/categories";
import TagsPage from "@/pages/tags";
import CommentsPage from "@/pages/comments";
import MediaPage from "@/pages/media";
import PagesPage from "@/pages/pages";
import PagesNewPage from "@/pages/pages-new";
import PagesEditPage from "@/pages/pages-edit";
import ContentTypesPage from "@/pages/content-types";
import ContentTypesBuilderPage from "@/pages/content-types-builder";
import ContentTypesListPage from "@/pages/content-types-list";
import ContentTypesNewPage from "@/pages/content-types-new";
import ContentTypesEditPage from "@/pages/content-types-edit";
import UsersPage from "@/pages/users";
import PluginsPage from "@/pages/plugins";
import PluginDetailPage from "@/pages/plugin-detail";
import RbacPage from "@/pages/rbac";
import CronsPage from "@/pages/crons";
import CronDetailPage from "@/pages/cron-detail";
import TenantsPage from "@/pages/tenants";
import WebhooksPage from "@/pages/webhooks";
import TokensPage from "@/pages/tokens";
import WorkflowsPage from "@/pages/workflows";
import WorkflowEditorPage from "@/pages/workflow-editor";
import WorkflowInstancesPage from "@/pages/workflow-instances";
import AuditPage from "@/pages/audit";
import OptionsPage from "@/pages/options";
import ReusableBlocksPage from "@/pages/reusable-blocks";

export function App() {
  return (
    <Providers>
      <Routes>
        <Route path="/auth/login" element={<LoginPage />} />
        <Route element={<AdminLayout />}>
          <Route index element={<Navigate to="dashboard" replace />} />
          <Route path="dashboard" element={<DashboardPage />} />
          <Route path="posts" element={<PostsPage />} />
          <Route path="posts/new" element={<PostsNewPage />} />
          <Route path="posts/:slug/edit" element={<PostsEditPage />} />
          <Route path="categories" element={<CategoriesPage />} />
          <Route path="tags" element={<TagsPage />} />
          <Route path="comments" element={<CommentsPage />} />
          <Route path="media" element={<MediaPage />} />
          <Route path="pages" element={<PagesPage />} />
          <Route path="pages/new" element={<PagesNewPage />} />
          <Route path="pages/:id/edit" element={<PagesEditPage />} />
          <Route path="content-types" element={<ContentTypesPage />} />
          <Route path="content-types/builder" element={<ContentTypesBuilderPage />} />
          <Route path="content-types/:singular" element={<ContentTypesListPage />} />
          <Route path="content-types/:singular/new" element={<ContentTypesNewPage />} />
          <Route path="content-types/:singular/:id/edit" element={<ContentTypesEditPage />} />
          <Route path="users" element={<UsersPage />} />
          <Route path="plugins" element={<PluginsPage />} />
          <Route path="plugins/:id" element={<PluginDetailPage />} />
          <Route path="rbac" element={<RbacPage />} />
          <Route path="crons" element={<CronsPage />} />
          <Route path="crons/:id" element={<CronDetailPage />} />
          <Route path="tenants" element={<TenantsPage />} />
          <Route path="webhooks" element={<WebhooksPage />} />
          <Route path="tokens" element={<TokensPage />} />
          <Route path="workflows" element={<WorkflowsPage />} />
          <Route path="workflows/editor" element={<WorkflowEditorPage />} />
          <Route path="workflows/instances" element={<WorkflowInstancesPage />} />
          <Route path="audit" element={<AuditPage />} />
          <Route path="options" element={<OptionsPage />} />
          <Route path="reusable-blocks" element={<ReusableBlocksPage />} />
        </Route>
      </Routes>
    </Providers>
  );
}
