import { Link as RouterLink } from "react-router-dom";

export default Link;

function Link({
  href,
  to,
  ...props
}: React.AnchorHTMLAttributes<HTMLAnchorElement> & {
  href?: string;
  to?: string;
  children?: React.ReactNode;
  replace?: boolean;
  prefetch?: boolean;
}) {
  return <RouterLink to={to ?? href ?? "/"} {...props} />;
}
