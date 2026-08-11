import type { StorefrontConfig } from './types';

export function StoreBrand({ storefront }: { storefront: StorefrontConfig }) {
  return (
    <div className="flex w-fit min-w-0 max-w-full flex-col items-center gap-2">
      <img
        alt={`${storefront.shop_name} Logo`}
        className="max-h-16 max-w-52 object-contain object-center"
        src={storefront.logo_url}
      />
      <p className="max-w-72 truncate text-center text-sm font-semibold text-slate-700">{storefront.shop_name}</p>
    </div>
  );
}
