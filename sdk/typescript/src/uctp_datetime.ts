/** The frozen Rust Chrono RFC3339 parser's accepted calendar and offset syntax. */
export function canonicalTimestamp(value: unknown): number {
    if (typeof value !== "string") return NaN;
    const match =
        /^(\d{4})-(\d{2})-(\d{2})[Tt ](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(?:([Zz])|([+\-\u2212])(\d{2}):(\d{2}))$/.exec(
            value,
        );
    if (!match || match[0].length !== value.length) return NaN;
    const year = Number(match[1]),
        month = Number(match[2]),
        day = Number(match[3]),
        hour = Number(match[4]),
        minute = Number(match[5]),
        second = Number(match[6]),
        offsetHour = Number(match[10] ?? 0),
        offsetMinute = Number(match[11] ?? 0),
        leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0),
        days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if (
        month < 1 ||
        month > 12 ||
        day < 1 ||
        day > days[month - 1] ||
        hour > 23 ||
        minute > 59 ||
        second > 60 ||
        offsetHour > 23 ||
        offsetMinute > 59
    )
        return NaN;
    const date = new Date(0);
    date.setUTCFullYear(year, month - 1, day);
    date.setUTCHours(
        hour,
        minute,
        Math.min(second, 59),
        Number((match[7] ?? "").slice(0, 3).padEnd(3, "0")),
    );
    const offset =
        (offsetHour * 60 + offsetMinute) * 60000 * (match[9] === "+" ? 1 : -1);
    return date.getTime() + (second === 60 ? 1000 : 0) - offset;
}
