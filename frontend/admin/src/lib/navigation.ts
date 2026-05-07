import {
  useNavigate,
  useLocation,
  useParams,
  useSearchParams as useRRSearchParams,
} from "react-router-dom";

export function useRouter() {
  const navigate = useNavigate();
  return {
    push: (path: string) => navigate(path),
    replace: (path: string) => navigate(path, { replace: true }),
    back: () => navigate(-1),
    forward: () => navigate(1),
    refresh: () => navigate(0),
  };
}

export function usePathname() {
  return useLocation().pathname;
}

export { useParams, useLocation };

export function useSearchParams(): URLSearchParams {
  return useRRSearchParams()[0];
}
