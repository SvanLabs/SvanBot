"use client";

// The home page: the sticky strip and one card per bot, fed by the dashboard's event stream.

import { FleetView } from "../src/components";
import { useFleet } from "../src/use-fleet";

export default function Page() {
  const { view, localNow } = useFleet();
  return <FleetView view={view} localNow={localNow} />;
}
