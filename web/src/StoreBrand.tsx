import type { StorefrontConfig } from './types';

export function StoreBrand({ storefront }: { storefront: StorefrontConfig }) {
  return (
    <div className="flex min-w-0 flex-col items-start gap-2">
      <img
        alt={`${storefront.shop_name} Logo`}
        className="max-h-16 max-w-52 object-contain object-left"
        src={storefront.logo_url}
      />
      <p className="max-w-72 truncate text-sm font-semibold text-slate-700">{storefront.shop_name}</p>
    </div>
  );
}
